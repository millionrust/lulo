//! Stable-GPUI platform diagnostic.
//!
//! This binary exercises the desktop contracts that must work before rmac
//! ports product applications to Linux. It intentionally stays small and does
//! not contain product behavior.

use gpui::{
    div, px, AppContext as _, ClipboardItem, Context, Entity, ExternalPaths,
    InteractiveElement as _, IntoElement, KeyBinding, ParentElement, PathPromptOptions, Render,
    SharedString, StatefulInteractiveElement as _, Styled, Window,
};
use gpui_component::{
    button::{Button, ButtonVariants as _},
    input::{Input, InputState},
    Sizable as _, StyledExt as _,
};
use rmac_ui::mac;

gpui::actions!(platform_lab, [CopyProbe, ReadProbe, OpenProbe]);

const CLIPBOARD_PROBE: &str = "rmac platform lab clipboard probe ✓";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CapabilityStatus {
    ExerciseHere,
    MissingFromStableApi,
}

impl CapabilityStatus {
    fn label(self) -> &'static str {
        match self {
            Self::ExerciseHere => "EXERCISE",
            Self::MissingFromStableApi => "UPSTREAM SPIKE",
        }
    }

    fn color(self) -> gpui::Hsla {
        match self {
            Self::ExerciseHere => gpui::rgb(0x248a3d).into(),
            Self::MissingFromStableApi => gpui::rgb(0xc9342f).into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Capability {
    id: &'static str,
    name: &'static str,
    status: CapabilityStatus,
    instruction: &'static str,
}

const CAPABILITIES: &[Capability] = &[
    Capability {
        id: "window",
        name: "Native window and GPU rendering",
        status: CapabilityStatus::ExerciseHere,
        instruction: "Resize, maximize, minimize, and move this window between displays.",
    },
    Capability {
        id: "input",
        name: "Text input and IME",
        status: CapabilityStatus::ExerciseHere,
        instruction: "Enter Latin, Indic, CJK, emoji, compose, and dead-key text.",
    },
    Capability {
        id: "clipboard",
        name: "Clipboard round trip",
        status: CapabilityStatus::ExerciseHere,
        instruction: "Copy the probe, read it back, then repeat with another application.",
    },
    Capability {
        id: "file-dialog",
        name: "Native file dialog",
        status: CapabilityStatus::ExerciseHere,
        instruction: "Open the chooser, select one file, and cancel once.",
    },
    Capability {
        id: "file-drop",
        name: "External file drop",
        status: CapabilityStatus::ExerciseHere,
        instruction: "Drop one or more files on the blue target.",
    },
    Capability {
        id: "scroll-scale",
        name: "Scrolling and fractional scaling",
        status: CapabilityStatus::ExerciseHere,
        instruction: "Scroll the list and check it at 100%, 125%, 150%, and 200%.",
    },
    Capability {
        id: "accessibility",
        name: "Programmatic accessibility",
        status: CapabilityStatus::MissingFromStableApi,
        instruction: "GPUI 0.2.2 exposes no accessibility tree API; test current upstream.",
    },
    Capability {
        id: "layer-shell",
        name: "Wayland layer-shell",
        status: CapabilityStatus::MissingFromStableApi,
        instruction: "GPUI 0.2.2 exposes no layer-shell API; test current upstream.",
    },
];

struct PlatformLab {
    input: Entity<InputState>,
    event: SharedString,
    file_selected: bool,
    dropped_path_count: usize,
    clipboard_result: ClipboardProbeResult,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ClipboardProbeResult {
    #[default]
    NotRead,
    ExactProbe,
    OtherText,
    NoText,
}

impl ClipboardProbeResult {
    fn summary(self) -> &'static str {
        match self {
            Self::NotRead => "No clipboard text read",
            Self::ExactProbe => "Built-in clipboard probe matched exactly",
            Self::OtherText => "Clipboard text received; content hidden",
            Self::NoText => "Clipboard did not contain text",
        }
    }
}

fn selected_file_summary(selected: bool) -> &'static str {
    if selected {
        "One file selected; path hidden"
    } else {
        "No file selected"
    }
}

fn dropped_paths_summary(count: usize) -> String {
    match count {
        0 => "No external files dropped".to_string(),
        1 => "One external path received; path hidden".to_string(),
        count => format!("{count} external paths received; paths hidden"),
    }
}

impl PlatformLab {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Type here: English, हिंदी, 中文, emoji, compose keys…")
        });

        cx.observe(&input, |this, _, cx| {
            this.event = "Text input changed".into();
            cx.notify();
        })
        .detach();

        cx.bind_keys([
            KeyBinding::new("cmd-shift-c", CopyProbe, Some("PlatformLab")),
            KeyBinding::new("ctrl-shift-c", CopyProbe, Some("PlatformLab")),
            KeyBinding::new("cmd-shift-v", ReadProbe, Some("PlatformLab")),
            KeyBinding::new("ctrl-shift-v", ReadProbe, Some("PlatformLab")),
            KeyBinding::new("cmd-o", OpenProbe, Some("PlatformLab")),
            KeyBinding::new("ctrl-o", OpenProbe, Some("PlatformLab")),
        ]);

        Self {
            input,
            event: "Ready — work through every probe and record the result".into(),
            file_selected: false,
            dropped_path_count: 0,
            clipboard_result: ClipboardProbeResult::NotRead,
        }
    }

    fn copy_probe(&mut self, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(CLIPBOARD_PROBE.to_string()));
        self.event = "Clipboard probe written".into();
        cx.notify();
    }

    fn read_probe(&mut self, cx: &mut Context<Self>) {
        self.clipboard_result = match cx.read_from_clipboard().and_then(|item| item.text()) {
            Some(text) if text == CLIPBOARD_PROBE => ClipboardProbeResult::ExactProbe,
            Some(_) => ClipboardProbeResult::OtherText,
            None => ClipboardProbeResult::NoText,
        };
        self.event = self.clipboard_result.summary().into();
        cx.notify();
    }

    fn open_probe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Select probe file".into()),
        });
        self.event = "File chooser opened".into();
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            let outcome = receiver.await;
            let _ = this.update_in(cx, |this, _, cx| {
                match outcome {
                    Ok(Ok(Some(paths))) => {
                        this.file_selected = !paths.is_empty();
                        this.event = "File chooser returned a path".into();
                    }
                    Ok(Ok(None)) => {
                        this.file_selected = false;
                        this.event = "File chooser cancelled".into();
                    }
                    Ok(Err(_)) => this.event = "File chooser returned an error".into(),
                    Err(_) => this.event = "File chooser channel closed unexpectedly".into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn render_probe_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex_1()
            .min_w(px(0.0))
            .v_flex()
            .gap_3()
            .child(section_title("Interactive probes"))
            .child(
                div()
                    .v_flex()
                    .gap_2()
                    .child(label("Text input and IME"))
                    .child(
                        div()
                            .border_1()
                            .border_color(mac::separator())
                            .rounded(px(7.0))
                            .px_2()
                            .py_1()
                            .child(Input::new(&self.input)),
                    ),
            )
            .child(
                div()
                    .v_flex()
                    .gap_2()
                    .child(label("Clipboard"))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new("copy-probe")
                                    .label("Copy probe")
                                    .primary()
                                    .with_size(gpui_component::Size::Small)
                                    .on_click(cx.listener(|this, _, _, cx| this.copy_probe(cx))),
                            )
                            .child(
                                Button::new("read-probe")
                                    .label("Read clipboard")
                                    .with_size(gpui_component::Size::Small)
                                    .on_click(cx.listener(|this, _, _, cx| this.read_probe(cx))),
                            ),
                    )
                    .child(value(self.clipboard_result.summary())),
            )
            .child(
                div()
                    .v_flex()
                    .gap_2()
                    .child(label("File chooser"))
                    .child(
                        Button::new("open-probe")
                            .label("Choose a file…")
                            .with_size(gpui_component::Size::Small)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_probe(window, cx)),
                            ),
                    )
                    .child(value(selected_file_summary(self.file_selected))),
            )
            .child(
                div()
                    .id("external-drop-target")
                    .h(px(92.0))
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .border_2()
                    .border_color(gpui::rgb(0x5aa9e6))
                    .rounded(px(9.0))
                    .bg(gpui::rgb(0xeaf5ff))
                    .text_size(px(12.0))
                    .text_color(mac::text_secondary())
                    .drag_over::<ExternalPaths>(|style, _, _, _| style.bg(gpui::rgb(0xcfeaff)))
                    .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                        this.dropped_path_count = paths.paths().len();
                        this.event =
                            format!("Received {} dropped path(s)", paths.paths().len()).into();
                        cx.notify();
                    }))
                    .child("Drop files here from the system file manager")
                    .child(value(dropped_paths_summary(self.dropped_path_count))),
            )
            .child(label("Scroll stress list"))
            .child(
                div()
                    .id("scroll-probe")
                    .h(px(180.0))
                    .w_full()
                    .overflow_y_scroll()
                    .border_1()
                    .border_color(mac::separator())
                    .rounded(px(7.0))
                    .children((1..=120).map(|row| {
                        div()
                            .h(px(28.0))
                            .flex()
                            .items_center()
                            .px_3()
                            .bg(if row % 2 == 0 {
                                gpui::rgb(0xf7f7f7).into()
                            } else {
                                mac::window()
                            })
                            .text_size(px(12.0))
                            .child(format!("Row {row:03} — smooth scroll and scale probe"))
                    })),
            )
    }

    fn render_capabilities(&self) -> impl IntoElement {
        div()
            .w(px(390.0))
            .min_w(px(300.0))
            .v_flex()
            .gap_3()
            .child(section_title("Stable GPUI 0.2.2 capability gate"))
            .children(CAPABILITIES.iter().map(|capability| {
                div()
                    .v_flex()
                    .gap_1()
                    .p_3()
                    .border_1()
                    .border_color(mac::separator())
                    .rounded(px(8.0))
                    .bg(mac::window())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(px(13.0))
                                    .font_weight(mac::SEMIBOLD)
                                    .text_color(mac::text())
                                    .child(capability.name),
                            )
                            .child(
                                div()
                                    .px_2()
                                    .py(px(2.0))
                                    .rounded_full()
                                    .bg(capability.status.color())
                                    .text_color(gpui::white())
                                    .text_size(px(9.0))
                                    .font_weight(mac::BOLD)
                                    .child(capability.status.label()),
                            ),
                    )
                    .child(value(capability.instruction))
            }))
    }
}

impl Render for PlatformLab {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("PlatformLab")
            .on_action(cx.listener(|this, _: &CopyProbe, _, cx| this.copy_probe(cx)))
            .on_action(cx.listener(|this, _: &ReadProbe, _, cx| this.read_probe(cx)))
            .on_action(cx.listener(|this, _: &OpenProbe, window, cx| this.open_probe(window, cx)))
            .size_full()
            .v_flex()
            .bg(mac::window())
            .text_color(mac::text())
            .child(rmac_ui::title_bar("rmac Platform Lab — GPUI 0.2.2"))
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .flex()
                    .gap_5()
                    .p_5()
                    .overflow_hidden()
                    .child(self.render_probe_panel(cx))
                    .child(self.render_capabilities()),
            )
            .child(
                div()
                    .h(px(34.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .px_4()
                    .border_t_1()
                    .border_color(mac::separator())
                    .bg(mac::chrome())
                    .text_size(px(11.0))
                    .text_color(mac::text_secondary())
                    .child(format!(
                        "{} / {} • {}",
                        std::env::consts::OS,
                        std::env::consts::ARCH,
                        self.event
                    )),
            )
    }
}

fn section_title(text: &'static str) -> impl IntoElement {
    div()
        .text_size(px(15.0))
        .font_weight(mac::SEMIBOLD)
        .text_color(mac::text())
        .child(text)
}

fn label(text: &'static str) -> impl IntoElement {
    div()
        .text_size(px(12.0))
        .font_weight(mac::MEDIUM)
        .text_color(mac::text_secondary())
        .child(text)
}

fn value(text: impl Into<SharedString>) -> impl IntoElement {
    div()
        .text_size(px(11.0))
        .text_color(mac::text_tertiary())
        .child(text.into())
}

fn main() {
    rmac_ui::boot("Platform Lab", 1120.0, 780.0, PlatformLab::new);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn capability_ids_are_unique() {
        let ids: HashSet<_> = CAPABILITIES
            .iter()
            .map(|capability| capability.id)
            .collect();
        assert_eq!(ids.len(), CAPABILITIES.len());
    }

    #[test]
    fn stable_api_records_phase_one_blockers() {
        for id in ["accessibility", "layer-shell"] {
            let capability = CAPABILITIES
                .iter()
                .find(|capability| capability.id == id)
                .expect("required gate exists");
            assert_eq!(capability.status, CapabilityStatus::MissingFromStableApi);
        }
    }

    #[test]
    fn core_desktop_probes_are_exercisable() {
        for id in [
            "window",
            "input",
            "clipboard",
            "file-dialog",
            "file-drop",
            "scroll-scale",
        ] {
            let capability = CAPABILITIES
                .iter()
                .find(|capability| capability.id == id)
                .expect("core probe exists");
            assert_eq!(capability.status, CapabilityStatus::ExerciseHere);
        }
    }

    #[test]
    fn evidence_summaries_never_include_clipboard_or_path_content() {
        assert_eq!(
            ClipboardProbeResult::ExactProbe.summary(),
            "Built-in clipboard probe matched exactly"
        );
        assert_eq!(
            ClipboardProbeResult::OtherText.summary(),
            "Clipboard text received; content hidden"
        );
        assert_eq!(
            selected_file_summary(true),
            "One file selected; path hidden"
        );
        assert_eq!(
            dropped_paths_summary(2),
            "2 external paths received; paths hidden"
        );
    }
}
