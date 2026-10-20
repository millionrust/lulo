//! Writing Tools for Notes, Text Editor and Mail (ADR 0024 feature 2), the
//! way macOS 26 offers them: Edit ▸ Writing Tools and the same submenu in
//! the text's right-click menu, then a small panel that shows Lulo
//! Intelligence's answer as it is written, with **Replace**, **Copy** and
//! **Try Again**.
//!
//! - Nothing changes the text until Replace, and Replace is one undoable
//!   edit: Edit ▸ Undo (⌘Z) gives the original back.
//! - **Stop** (or Escape) cancels mid-answer: the service is told at once
//!   and the text is never touched.
//! - The answer streams from `org.rmac.Intelligence1` on a background
//!   executor ([`rmac_intelligence::client::write`]); the UI thread only
//!   appends the text it is handed. The editor never waits.
//! - With Lulo Intelligence off, or no model on disk, every Writing Tools
//!   row is hidden ([`publish_menu_state`], [`text_menu`]), as the Mac
//!   hides them without Apple Intelligence. A task this PC's model is too
//!   small for ([`rmac_intelligence::writing::Spec::min_tier`]) is shown
//!   disabled, and the panel says "Needs a faster PC".
//!
//! The host app owns one [`WritingTools`] entity per window, renders it as
//! the last child of its root, routes the `writing_tools::` actions to it
//! with [`on_actions`], and says which text they act on with a
//! [`Target`].

use std::ops::Range;
use std::rc::Rc;

use gpui::{
    accesskit, div, prelude::FluentBuilder as _, px, App, ClipboardItem, Context, Entity,
    FocusHandle, Focusable, FontWeight, Global, InteractiveElement, IntoElement, KeyBinding,
    ParentElement as _, Render, Role, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window,
};
use rmac_intelligence::client::{ClientError, WriteDone};
use rmac_intelligence::manifest::Tier;
use rmac_intelligence::writing::{self, Refusal};
use rmac_intelligence::Task;
use rmac_ui::{mac, Button, EditableText, NativeMenu};

pub mod text_menu;
pub use text_menu::{context_menu, text_menu, TextMenuSpec, TextState};

gpui::actions!(
    writing_tools,
    [
        /// Edit ▸ Writing Tools ▸ Show Writing Tools: the panel with every
        /// task to choose from.
        ShowWritingTools,
        Proofread,
        Rewrite,
        MakeFriendly,
        MakeProfessional,
        MakeConcise,
        Summarise,
        CreateKeyPoints,
        MakeList,
        /// Escape in the panel: stop, or close.
        Dismiss,
        /// Return in the panel: Replace, once the answer is done.
        Accept,
    ]
);

/// The submenu's own action, which [`publish_menu_state`] hides.
pub const MENU_ACTION: &str = "writing_tools::WritingToolsMenu";
const SHOW_ACTION: &str = "writing_tools::ShowWritingTools";
const KEY_CONTEXT: &str = "WritingTools";

/// Each task's menu action, in menu order.
pub const TASK_ACTIONS: [(Task, &str); 8] = [
    (Task::Proofread, "writing_tools::Proofread"),
    (Task::Rewrite, "writing_tools::Rewrite"),
    (Task::Friendly, "writing_tools::MakeFriendly"),
    (Task::Professional, "writing_tools::MakeProfessional"),
    (Task::Concise, "writing_tools::MakeConcise"),
    (Task::Summarise, "writing_tools::Summarise"),
    (Task::KeyPoints, "writing_tools::CreateKeyPoints"),
    (Task::List, "writing_tools::MakeList"),
];

fn task_action(task: Task) -> Box<dyn gpui::Action> {
    match task {
        Task::Proofread => Box::new(Proofread),
        Task::Rewrite => Box::new(Rewrite),
        Task::Friendly => Box::new(MakeFriendly),
        Task::Professional => Box::new(MakeProfessional),
        Task::Concise => Box::new(MakeConcise),
        Task::Summarise => Box::new(Summarise),
        Task::KeyPoints => Box::new(CreateKeyPoints),
        Task::List | Task::Intent => Box::new(MakeList),
    }
}

fn label(task: Task) -> &'static str {
    writing::spec(task).map_or("", |spec| spec.menu_label)
}

fn title(task: Task) -> &'static str {
    writing::spec(task).map_or("Writing Tools", |spec| spec.title)
}

// ---- Availability --------------------------------------------------------------

/// Whether Writing Tools are offered in this session, and on which model.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Availability {
    /// Lulo Intelligence is on, this PC can run it and the model is on
    /// disk.
    pub shown: bool,
    /// The model the service loads, for the per-task gate.
    pub tier: Option<Tier>,
}

impl Availability {
    /// Whether `task` runs on this PC's model.
    pub fn offered(&self, task: Task) -> bool {
        self.shown
            && writing::spec(task)
                .zip(self.tier)
                .is_some_and(|(spec, tier)| spec.offered_on(tier))
    }
}

struct AvailabilityGlobal(Availability);

impl Global for AvailabilityGlobal {}

/// What [`refresh`] last found; hidden until it has looked.
pub fn availability(cx: &App) -> Availability {
    cx.try_global::<AvailabilityGlobal>()
        .map(|global| global.0)
        .unwrap_or_default()
}

/// The check behind [`refresh`]: the user's setting, the hardware gate and
/// the model's stamp (no hashing). Blocking but cheap (one small file, two
/// `/proc` reads, two `stat`s): run it off the UI thread.
pub fn probe() -> Availability {
    if !cfg!(target_os = "linux") {
        // The service runs on Linux only (ADR 0024 phase 5 is Windows).
        return Availability::default();
    }
    let config = rmac_intelligence::config::Config::load();
    if !config.enabled {
        return Availability::default();
    }
    // The behaviour scenarios' fixture engine needs no model file.
    if std::env::var("RMAC_INTELLIGENCE_ENGINE").is_ok_and(|engine| engine == "fixture") {
        let tier = match std::env::var("RMAC_INTELLIGENCE_FIXTURE_TIER").as_deref() {
            Ok("tiny") => Tier::Tiny,
            _ => Tier::Standard,
        };
        return Availability {
            shown: true,
            tier: Some(tier),
        };
    }
    let facts = rmac_intelligence::gate::current_facts();
    let Some(tier) = config.tier(&facts) else {
        return Availability::default();
    };
    let present = rmac_intelligence::paths::models_dir()
        .is_some_and(|models| rmac_intelligence::verify::is_present(&models, tier.model()));
    Availability {
        shown: present,
        tier: present.then_some(tier),
    }
}

/// Look again, off the UI thread, and update the menus and every window
/// when the answer changed. Hosts call it when a window becomes active, so
/// turning Lulo Intelligence on in System Settings shows Writing Tools the
/// next time the app is used; nothing polls.
pub fn refresh(cx: &mut App) {
    let task = cx.background_executor().spawn(async { probe() });
    cx.spawn(async move |cx| {
        let found = task.await;
        cx.update(|cx| {
            // The menus may have been installed since the last look.
            rmac_ui::set_menu_hidden(MENU_ACTION, !found.shown, cx);
            if cx.try_global::<AvailabilityGlobal>().map(|global| global.0) != Some(found) {
                cx.set_global(AvailabilityGlobal(found));
                cx.refresh_windows();
            }
        })
    })
    .detach();
}

/// Bind the panel's keys and look for the first time. Call once at start,
/// before or after the app's menus are installed (the menus hide Writing
/// Tools from the host's first [`publish_menu_state`] on).
pub fn init(cx: &mut App) {
    cx.set_global(AvailabilityGlobal(Availability::default()));
    rmac_ui::set_menu_hidden(MENU_ACTION, true, cx);
    rmac_ui::bind_keys(
        cx,
        [
            KeyBinding::new("escape", Dismiss, Some(KEY_CONTEXT)),
            KeyBinding::new("enter", Accept, Some(KEY_CONTEXT)),
        ],
    );
    refresh(cx);
}

/// Edit ▸ Writing Tools for the key window: hidden while unavailable, each
/// command enabled while `editable` (there is text to work on) and its task
/// runs on this PC's model. Call from the host's menu-state publishing.
pub fn publish_menu_state(editable: bool, cx: &mut App) {
    let found = availability(cx);
    rmac_ui::set_menu_hidden(MENU_ACTION, !found.shown, cx);
    rmac_ui::set_menu_enabled(SHOW_ACTION, found.shown && editable, cx);
    for (task, action) in TASK_ACTIONS {
        rmac_ui::set_menu_enabled(action, editable && found.offered(task), cx);
    }
}

// ---- The text Writing Tools act on ---------------------------------------------

/// A text field the panel reads and replaces in, through its own undoable
/// edit path.
pub trait TextTarget {
    fn text(&self, cx: &App) -> String;
    fn selection(&self, cx: &App) -> Range<usize>;
    fn replace(&self, range: Range<usize>, text: &str, window: &mut Window, cx: &mut App);
    fn select(&self, range: Range<usize>, window: &mut Window, cx: &mut App);
    fn focus(&self, window: &mut Window, cx: &mut App);
}

impl<T: EditableText> TextTarget for Entity<T> {
    fn text(&self, cx: &App) -> String {
        self.read(cx).editable_text()
    }

    fn selection(&self, cx: &App) -> Range<usize> {
        self.read(cx).editable_selection()
    }

    fn replace(&self, range: Range<usize>, text: &str, window: &mut Window, cx: &mut App) {
        self.update(cx, |field, cx| {
            field.replace_editable_range(range, text, window, cx)
        });
    }

    fn select(&self, range: Range<usize>, window: &mut Window, cx: &mut App) {
        self.update(cx, |field, cx| {
            field.select_editable_range(range, window, cx)
        });
    }

    fn focus(&self, window: &mut Window, cx: &mut App) {
        self.update(cx, |field, cx| field.focus_editable(window, cx));
    }
}

pub type Target = Rc<dyn TextTarget>;

/// Names the text a Writing Tools command acts on, when one fires.
type TargetFinder = Rc<dyn Fn(&mut Window, &mut App) -> Option<Target>>;

/// The [`Target`] for an editable field.
pub fn target<T: EditableText>(field: &Entity<T>) -> Target {
    Rc::new(field.clone())
}

/// The part of `text` a task works on: the selection, or the whole text
/// when nothing is selected (as the Mac does). Snapped to character
/// boundaries.
pub fn working_range(text: &str, selection: Range<usize>) -> Range<usize> {
    let end = selection.end.min(text.len());
    let start = selection.start.min(end);
    if start == end || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
        return 0..text.len();
    }
    start..end
}

/// Where `original` is now: still at `range`, or (the text was edited
/// meanwhile) its one occurrence nearest to `range`. `None` once it is
/// gone, when Replace would change the wrong words.
pub fn locate(text: &str, range: &Range<usize>, original: &str) -> Option<Range<usize>> {
    if text.get(range.clone()) == Some(original) {
        return Some(range.clone());
    }
    if original.is_empty() {
        return None;
    }
    text.match_indices(original)
        .map(|(start, _)| start..start + original.len())
        .min_by_key(|found| found.start.abs_diff(range.start))
}

// ---- Context menus -------------------------------------------------------------

/// Writing Tools appended to a native text menu, after a separator, while
/// it is available and the field is editable.
fn append_native_writing_tools(menu: NativeMenu, editable: bool, cx: &App) -> NativeMenu {
    let found = availability(cx);
    if !(found.shown && editable) {
        return menu;
    }
    let mut tools = NativeMenu::new().menu("Show Writing Tools", Box::new(ShowWritingTools));
    for (index, (task, _)) in TASK_ACTIONS.iter().enumerate() {
        if matches!(index, 0 | 2 | 5) {
            tools = tools.separator();
        }
        tools = tools.menu_with_disabled(label(*task), !found.offered(*task), task_action(*task));
    }
    menu.separator().submenu("Writing Tools", tools)
}

/// The same Writing Tools rows for a Lulo-drawn [`rmac_ui::ContextMenu`]
/// (Text Editor's rich-text body): appended after a separator while
/// available and `editable`.
pub fn context_menu_rows(
    menu: rmac_ui::ContextMenu,
    position: gpui::Point<gpui::Pixels>,
    editable: bool,
    cx: &App,
) -> rmac_ui::ContextMenu {
    let found = availability(cx);
    if !(found.shown && editable) {
        return menu;
    }
    let mut tools =
        rmac_ui::ContextMenu::new(position).item("Show Writing Tools", Box::new(ShowWritingTools));
    for (index, (task, _)) in TASK_ACTIONS.iter().enumerate() {
        if matches!(index, 0 | 2 | 5) {
            tools = tools.separator();
        }
        tools = if found.offered(*task) {
            tools.item(label(*task), task_action(*task))
        } else {
            tools.disabled_item(label(*task), task_action(*task))
        };
    }
    menu.separator().submenu("Writing Tools", tools)
}

/// Route the `writing_tools::` actions on `element` to `panel`, for the
/// text `target` names when one fires (`None` when nothing editable has the
/// selection, which leaves the action unhandled).
pub fn on_actions<E: InteractiveElement>(
    element: E,
    panel: &Entity<WritingTools>,
    target: impl Fn(&mut Window, &mut App) -> Option<Target> + 'static,
) -> E {
    let target: TargetFinder = Rc::new(target);
    let run = {
        let panel = panel.clone();
        let target = target.clone();
        move |task: Option<Task>, window: &mut Window, cx: &mut App| {
            let Some(field) = target(window, cx) else {
                return;
            };
            panel.update(cx, |panel, cx| match task {
                Some(task) => panel.start(task, field, window, cx),
                None => panel.choose(field, window, cx),
            });
        }
    };
    let run = Rc::new(run);
    macro_rules! route {
        ($element:expr, $action:ty, $task:expr) => {{
            let run = run.clone();
            $element.on_action(move |_: &$action, window, cx| run($task, window, cx))
        }};
    }
    let element = route!(element, ShowWritingTools, None);
    let element = route!(element, Proofread, Some(Task::Proofread));
    let element = route!(element, Rewrite, Some(Task::Rewrite));
    let element = route!(element, MakeFriendly, Some(Task::Friendly));
    let element = route!(element, MakeProfessional, Some(Task::Professional));
    let element = route!(element, MakeConcise, Some(Task::Concise));
    let element = route!(element, Summarise, Some(Task::Summarise));
    let element = route!(element, CreateKeyPoints, Some(Task::KeyPoints));
    route!(element, MakeList, Some(Task::List))
}

// ---- The panel -----------------------------------------------------------------

enum Event {
    Delta(String),
    Done(Result<WriteDone, ClientError>),
}

#[derive(Clone, Debug, PartialEq)]
enum Phase {
    Closed,
    /// Show Writing Tools: the tasks to choose from.
    Choosing,
    /// Asked; the model is reading the text (no answer yet).
    Reading,
    /// The answer so far.
    Writing,
    Done {
        capped: bool,
    },
    Failed(String),
}

/// The one Writing Tools panel of a window.
pub struct WritingTools {
    focus: FocusHandle,
    phase: Phase,
    target: Option<Target>,
    task: Task,
    /// The text worked on, exactly as it was, and where it was.
    original: String,
    range: Range<usize>,
    answer: String,
    attempt: u32,
    /// Counts runs, so a stopped run's late events are ignored.
    run: u64,
    cancel: Option<async_channel::Sender<()>>,
    copied: bool,
}

impl Focusable for WritingTools {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// The message for a request that did not finish.
fn failure_message(task: Task, error: &ClientError) -> String {
    let words = writing::spec(task).map_or(0, |spec| spec.max_input_words());
    match error {
        ClientError::TooLong => {
            format!("Select up to about {words} words to use {}.", label(task))
        }
        ClientError::NeedsFasterPc => format!("{} needs a faster PC.", label(task)),
        ClientError::Off => "Turn on Lulo Intelligence in System Settings.".into(),
        ClientError::NotDownloaded => {
            "Download the Lulo Intelligence model in System Settings.".into()
        }
        ClientError::LowMemory => {
            "There is not enough free memory right now. Close some windows and try again.".into()
        }
        ClientError::NotSupported => "Lulo Intelligence is not available on this PC.".into(),
        ClientError::Unavailable | ClientError::Refused | ClientError::Failed => {
            "Writing Tools could not finish. Try again.".into()
        }
        ClientError::Cancelled => "Stopped.".into(),
    }
}

impl WritingTools {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            phase: Phase::Closed,
            target: None,
            task: Task::Proofread,
            original: String::new(),
            range: 0..0,
            answer: String::new(),
            attempt: 0,
            run: 0,
            cancel: None,
            copied: false,
        }
    }

    /// Whether the panel is showing (the host may hide its own popovers).
    pub fn is_open(&self) -> bool {
        self.phase != Phase::Closed
    }

    /// Whether an answer is being written.
    pub fn is_working(&self) -> bool {
        matches!(self.phase, Phase::Reading | Phase::Writing)
    }

    fn take_target(&mut self, field: Target, cx: &App) {
        let text = field.text(cx);
        self.range = working_range(&text, field.selection(cx));
        self.original = text[self.range.clone()].to_owned();
        self.target = Some(field);
    }

    /// Show Writing Tools: the panel with every task.
    pub fn choose(&mut self, field: Target, window: &mut Window, cx: &mut Context<Self>) {
        if !availability(cx).shown {
            return;
        }
        self.stop();
        self.take_target(field, cx);
        self.phase = Phase::Choosing;
        self.answer.clear();
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Run `task` on the field's selection (or all of its text).
    pub fn start(
        &mut self,
        task: Task,
        field: Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let found = availability(cx);
        if !found.shown {
            return;
        }
        self.stop();
        self.take_target(field, cx);
        self.attempt = 0;
        self.task = task;
        window.focus(&self.focus, cx);
        if !found.offered(task) {
            self.fail(failure_message(task, &ClientError::NeedsFasterPc), cx);
            return;
        }
        self.ask(cx);
    }

    fn fail(&mut self, message: String, cx: &mut Context<Self>) {
        self.phase = Phase::Failed(message);
        cx.notify();
    }

    /// Send the request; the answer arrives through [`Self::apply`].
    fn ask(&mut self, cx: &mut Context<Self>) {
        let task = self.task;
        match writing::worth_asking(task, &self.original) {
            Ok(()) => {}
            Err(Refusal::TooLong(_)) => {
                return self.fail(failure_message(task, &ClientError::TooLong), cx);
            }
            Err(_) => return self.fail("There is no text to work on.".into(), cx),
        }
        self.run += 1;
        let run = self.run;
        self.answer.clear();
        self.copied = false;
        self.phase = Phase::Reading;
        let (cancel, cancelled) = async_channel::bounded(1);
        self.cancel = Some(cancel);
        let (events, incoming) = async_channel::unbounded::<Event>();
        let text = self.original.clone();
        let attempt = self.attempt;
        rmac_ui::trace_mark("writing_tools_asked");
        cx.background_executor()
            .spawn(async move {
                let deltas = events.clone();
                let result = rmac_intelligence::client::write(
                    task,
                    &text,
                    attempt,
                    cancelled,
                    move |piece| {
                        let _ = deltas.try_send(Event::Delta(piece.to_owned()));
                    },
                )
                .await;
                let _ = events.try_send(Event::Done(result));
            })
            .detach();
        cx.spawn(async move |this, cx| {
            while let Ok(first) = incoming.recv().await {
                // Everything that arrived meanwhile goes into one frame.
                let mut batch = vec![first];
                while let Ok(more) = incoming.try_recv() {
                    batch.push(more);
                }
                let open = this
                    .update(cx, |this, cx| {
                        for event in batch {
                            this.apply(run, event, cx);
                        }
                        this.run == run
                    })
                    .unwrap_or(false);
                if !open {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }

    fn apply(&mut self, run: u64, event: Event, cx: &mut Context<Self>) {
        if run != self.run || !self.is_working() {
            return;
        }
        match event {
            Event::Delta(piece) => {
                if self.answer.is_empty() {
                    rmac_ui::trace_mark("writing_tools_first_delta");
                }
                self.answer.push_str(&piece);
                self.phase = Phase::Writing;
            }
            Event::Done(Ok(done)) => {
                self.answer = done.text;
                self.cancel = None;
                rmac_ui::trace_mark(&format!(
                    "writing_tools_done:first_token_ms={:.0}:output_tokens={}:decode_tok_s={:.1}",
                    done.timing.first_token_ms, done.timing.output_tokens, done.timing.decode_tok_s
                ));
                self.phase = if self.answer.trim().is_empty() {
                    Phase::Failed(failure_message(self.task, &ClientError::Failed))
                } else {
                    Phase::Done {
                        capped: done.capped,
                    }
                };
            }
            Event::Done(Err(error)) => {
                self.cancel = None;
                rmac_ui::trace_mark("writing_tools_failed");
                self.phase = Phase::Failed(failure_message(self.task, &error));
            }
        }
        cx.notify();
    }

    /// Stop a running answer (the service is told at once).
    fn stop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.try_send(());
            rmac_ui::trace_mark("writing_tools_cancelled");
        }
        self.run += 1;
    }

    /// Stop and close, giving focus back to the text.
    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.stop();
        self.phase = Phase::Closed;
        self.answer.clear();
        if let Some(field) = self.target.take() {
            field.focus(window, cx);
        }
        cx.notify();
    }

    /// Try Again: a new answer for the same text.
    fn try_again(&mut self, cx: &mut Context<Self>) {
        if self.target.is_none() {
            return;
        }
        self.stop();
        self.attempt += 1;
        self.ask(cx);
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        if matches!(self.phase, Phase::Done { .. }) {
            cx.write_to_clipboard(ClipboardItem::new_string(self.answer.clone()));
            self.copied = true;
            cx.notify();
        }
    }

    /// Replace the text worked on with the answer, as one undoable edit,
    /// and select it, as the Mac does.
    fn replace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.phase, Phase::Done { .. }) {
            return;
        }
        let Some(field) = self.target.clone() else {
            return;
        };
        let text = field.text(cx);
        let Some(range) = locate(&text, &self.range, &self.original) else {
            return self.fail(
                "The text changed. Copy the result instead, or try again.".into(),
                cx,
            );
        };
        let replacement = writing::rewrap(&self.original, &self.answer);
        let start = range.start;
        field.replace(range, &replacement, window, cx);
        field.select(start..start + replacement.len(), window, cx);
        rmac_ui::trace_mark("writing_tools_replaced");
        self.close(window, cx);
    }

    fn render_header(&self) -> impl IntoElement {
        let heading = match &self.phase {
            Phase::Choosing => "Writing Tools",
            _ => title(self.task),
        };
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(mac::text())
                    .child(heading),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(mac::text_secondary())
                    .child("Lulo Intelligence"),
            )
    }

    fn render_chooser(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let found = availability(cx);
        let entity = cx.entity();
        let groups: [&[Task]; 3] = [
            &[Task::Proofread, Task::Rewrite],
            &[Task::Friendly, Task::Professional, Task::Concise],
            &[Task::Summarise, Task::KeyPoints, Task::List],
        ];
        let mut column = div().flex().flex_col().gap(px(8.0));
        let mut gated = false;
        for group in groups {
            let mut row = div().flex().flex_wrap().gap(px(8.0));
            for task in group {
                let task = *task;
                let offered = found.offered(task);
                gated |= !offered;
                let entity = entity.clone();
                row = row.child(
                    Button::new(("writing-tools-task", task as usize), title(task))
                        .small()
                        .disabled(!offered)
                        .when(!offered, |button| button.tooltip("Needs a faster PC"))
                        .on_click(move |_, window, cx| {
                            entity.update(cx, |this, cx| {
                                if let Some(field) = this.target.clone() {
                                    this.start(task, field, window, cx);
                                }
                            });
                        }),
                );
            }
            column = column.child(row);
        }
        column.when(gated, |column| {
            column.child(
                div()
                    .text_size(px(11.0))
                    .text_color(mac::text_secondary())
                    .child("Some tools need a faster PC."),
            )
        })
    }

    fn render_result(&self) -> impl IntoElement {
        let status: Option<SharedString> = match &self.phase {
            Phase::Reading => Some("Reading the text…".into()),
            Phase::Writing => Some("Writing…".into()),
            Phase::Done { capped: true } => Some("The answer was cut short.".into()),
            _ => None,
        };
        let failed = match &self.phase {
            Phase::Failed(message) => Some(message.clone()),
            _ => None,
        };
        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .when(!self.answer.is_empty() && failed.is_none(), |body| {
                body.child(
                    div()
                        .id("writing-tools-result")
                        .role(Role::Status)
                        .aria_label(SharedString::from(self.answer.clone()))
                        .max_h(px(240.0))
                        .overflow_y_scroll()
                        .text_size(px(13.0))
                        .line_height(px(18.0))
                        .text_color(mac::text())
                        .child(self.answer.clone()),
                )
            })
            .when_some(failed, |body, message| {
                body.child(
                    div()
                        .id("writing-tools-error")
                        .role(Role::Status)
                        .aria_label(SharedString::from(message.clone()))
                        .a11y_synthetic_children(|builder| {
                            builder.parent_node().set_live(accesskit::Live::Polite);
                        })
                        .text_size(px(13.0))
                        .text_color(mac::text())
                        .child(message),
                )
            })
            .when_some(status, |body, status| {
                body.child(
                    div()
                        .id("writing-tools-status")
                        .role(Role::Status)
                        .aria_label(status.clone())
                        .a11y_synthetic_children(|builder| {
                            builder.parent_node().set_live(accesskit::Live::Polite);
                        })
                        .text_size(px(12.0))
                        .text_color(mac::text_secondary())
                        .child(status),
                )
            })
    }

    fn render_buttons(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let button = |id: &'static str, text: &'static str| {
            let entity = entity.clone();
            (Button::new(id, text).small(), entity)
        };
        let mut row = div().flex().items_center().gap(px(8.0));
        match &self.phase {
            Phase::Reading | Phase::Writing => {
                let (stop, entity) = button("writing-tools-stop", "Stop");
                row = row.child(stop.on_click(move |_, window, cx| {
                    entity.update(cx, |this, cx| this.close(window, cx));
                }));
            }
            Phase::Done { .. } => {
                let (again, again_entity) = button("writing-tools-try-again", "Try Again");
                let (copy, copy_entity) = button(
                    "writing-tools-copy",
                    if self.copied { "Copied" } else { "Copy" },
                );
                let (replace, replace_entity) = button("writing-tools-replace", "Replace");
                row = row
                    .child(again.on_click(move |_, _, cx| {
                        again_entity.update(cx, |this, cx| this.try_again(cx));
                    }))
                    .child(copy.on_click(move |_, _, cx| {
                        copy_entity.update(cx, |this, cx| this.copy(cx));
                    }))
                    .child(replace.primary().on_click(move |_, window, cx| {
                        replace_entity.update(cx, |this, cx| this.replace(window, cx));
                    }));
            }
            Phase::Failed(_) | Phase::Choosing | Phase::Closed => {
                if matches!(self.phase, Phase::Failed(_)) && !self.original.trim().is_empty() {
                    let (again, again_entity) = button("writing-tools-try-again", "Try Again");
                    row = row.child(again.on_click(move |_, _, cx| {
                        again_entity.update(cx, |this, cx| this.try_again(cx));
                    }));
                }
                let (close, entity) = button("writing-tools-close", "Close");
                row = row.child(close.on_click(move |_, window, cx| {
                    entity.update(cx, |this, cx| this.close(window, cx));
                }));
            }
        }
        row
    }
}

impl Render for WritingTools {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.phase == Phase::Closed {
            return div().id("writing-tools").into_any_element();
        }
        let body = if self.phase == Phase::Choosing {
            self.render_chooser(cx).into_any_element()
        } else {
            self.render_result().into_any_element()
        };
        let panel = div()
            .id("writing-tools")
            .role(Role::Dialog)
            .aria_label("Writing Tools")
            .track_focus(&self.focus)
            .key_context(KEY_CONTEXT)
            .on_action(cx.listener(|this, _: &Dismiss, window, cx| this.close(window, cx)))
            .on_action(cx.listener(|this, _: &Accept, window, cx| this.replace(window, cx)))
            .occlude()
            .absolute()
            .top(px(64.0))
            .right(px(16.0))
            .w(px(380.0))
            .px(px(14.0))
            .py(px(12.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .rounded(px(mac::radius_popover()))
            .border_1()
            .border_color(mac::separator())
            .bg(mac::material_popover())
            .shadow(mac::menu_shadow())
            .child(self.render_header())
            .child(body)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(11.0))
                            .text_color(mac::text_secondary())
                            .child("Processed on this PC."),
                    )
                    .child(self.render_buttons(cx)),
            );
        gpui::deferred(panel).with_priority(2).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_selection_or_else_the_whole_text_is_worked_on() {
        let text = "Hello wrld. Bye.";
        assert_eq!(working_range(text, 6..11), 6..11);
        assert_eq!(working_range(text, 3..3), 0..text.len());
        assert_eq!(working_range(text, 20..30), 0..text.len());
        // Never inside a character.
        assert_eq!(working_range("é a", 1..3), 0..4);
    }

    #[test]
    fn replace_finds_the_original_even_after_edits_elsewhere() {
        let original = "teh cat";
        assert_eq!(locate("teh cat sat", &(0..7), original), Some(0..7));
        // Text typed before it moved it along.
        assert_eq!(locate("Oh, teh cat sat", &(0..7), original), Some(4..11));
        // The nearest of several.
        assert_eq!(
            locate("teh cat, and later teh cat", &(18..25), original),
            Some(19..26)
        );
        assert_eq!(locate("the cat sat", &(0..7), original), None);
    }

    #[test]
    fn availability_gates_each_task_by_tier() {
        let tiny = Availability {
            shown: true,
            tier: Some(Tier::Tiny),
        };
        let standard = Availability {
            shown: true,
            tier: Some(Tier::Standard),
        };
        for task in Task::WRITING {
            assert!(standard.offered(task), "{task:?}");
            let spec = writing::spec(task).unwrap();
            assert_eq!(tiny.offered(task), spec.min_tier == Tier::Tiny, "{task:?}");
        }
        assert!(!Availability::default().offered(Task::Proofread));
        assert!(!standard.offered(Task::Intent));
    }

    #[test]
    fn every_task_has_one_menu_action() {
        assert_eq!(TASK_ACTIONS.len(), Task::WRITING.len());
        for ((task, name), expected) in TASK_ACTIONS.iter().zip(Task::WRITING) {
            assert_eq!(*task, expected);
            assert_eq!(task_action(*task).name(), *name);
        }
    }

    #[test]
    fn failures_say_what_to_do() {
        assert_eq!(
            failure_message(Task::Summarise, &ClientError::NeedsFasterPc),
            "Summarise needs a faster PC."
        );
        assert!(failure_message(Task::Proofread, &ClientError::TooLong).contains("480 words"));
        assert!(failure_message(Task::Rewrite, &ClientError::Off).contains("System Settings"));
    }
}
