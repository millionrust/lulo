use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    mpsc::{sync_channel, SyncSender},
    Arc, Mutex,
};
use std::thread::JoinHandle;

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::Processor;
use portable_pty::{
    native_pty_system, Child, ChildKiller, CommandBuilder, ExitStatus, MasterPty, PtySize,
};

use crate::emulator::{advance_filtered_output, terminal_config, TermSize};
use crate::job::{ForegroundJobSource, SessionJobState};
use crate::output_filter::OutputFilter;
use crate::paste::{has_unsafe_unbracketed_control, logical_line_count, prepare as prepare_paste};

use crate::shell_integration::{
    CommandRangeKind, MarkerPosition, PromptDirection, SessionShellState,
};
use crate::title::SessionTitle;
use crate::ui_state::{Selection, SessionUiState};
use crate::working_directory::SessionDirectory;

pub(super) type RedrawSender = async_channel::Sender<()>;

const SESSION_WORKER_STACK_BYTES: usize = 512 * 1024;
#[cfg(test)]
const SESSION_WORKERS_PER_TAB: usize = 2;
static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

type SharedWriter = Arc<Mutex<Box<dyn Write + Send>>>;

#[derive(Clone, Default)]
pub(super) struct EventProxy {
    writer: Option<SharedWriter>,
    write_failed: Option<Arc<AtomicBool>>,
    title: SessionTitle,
    redraw: Option<RedrawSender>,
}

impl EventProxy {
    fn connected(
        writer: SharedWriter,
        write_failed: Arc<AtomicBool>,
        title: SessionTitle,
        redraw: RedrawSender,
    ) -> Self {
        Self {
            writer: Some(writer),
            write_failed: Some(write_failed),
            title,
            redraw: Some(redraw),
        }
    }
}

impl EventListener for EventProxy {
    fn send_event(&self, event: Event) {
        match event {
            Event::PtyWrite(text) => {
                let Some(writer) = self.writer.as_ref() else {
                    return;
                };
                let result = writer.lock().map_err(|_| ()).and_then(|mut writer| {
                    writer
                        .write_all(text.as_bytes())
                        .and_then(|_| writer.flush())
                        .map_err(|_| ())
                });
                if result.is_err() {
                    if let Some(write_failed) = self.write_failed.as_ref() {
                        write_failed.store(true, Ordering::Release);
                    }
                }
            }
            Event::Title(title) if self.title.set(Some(&title)) => {
                if let Some(redraw) = self.redraw.as_ref() {
                    request_redraw(redraw);
                }
            }
            Event::ResetTitle if self.title.set(None) => {
                if let Some(redraw) = self.redraw.as_ref() {
                    request_redraw(redraw);
                }
            }
            _ => {}
        }
    }
}

fn request_redraw(redraw: &RedrawSender) {
    let _ = redraw.try_send(());
}

fn shell_program(configured: Option<String>) -> String {
    configured
        .filter(|shell| !shell.trim().is_empty())
        .unwrap_or_else(default_shell)
}

/// The shell Terminal starts when the user has not chosen one: `$SHELL` on
/// Unix, PowerShell on Windows (ConPTY; see ADR 0023 phase 2). Named by
/// program only — `CommandBuilder`/ConPTY resolve it through `PATH`, same as
/// `cmd.exe` would.
#[cfg(unix)]
fn default_shell() -> String {
    "/bin/sh".to_string()
}

#[cfg(windows)]
fn default_shell() -> String {
    "powershell.exe".to_string()
}

/// Terminal's own default title when nothing else claims it: "<user> —
/// -<shell>", matching the Mac's "jake — -zsh" (Window ▸ Title uses: Shell,
/// its default for a plain, idle window) — never the current directory's
/// name, which `Session::tab_title` fell back to display instead (UIA-02).
/// The leading "-" mirrors how a login shell's own argv[0] reads in `ps`.
/// `user` is read from `$USER`/`$LOGNAME` by the caller, not here, so this
/// stays a pure function the tests below can drive directly.
fn default_shell_identity(user: Option<&str>, shell: &str) -> Option<String> {
    let user = user.filter(|value| !value.trim().is_empty())?;
    let shell_name = std::path::Path::new(shell).file_name()?.to_str()?;
    (!shell_name.is_empty()).then(|| format!("{user} — -{shell_name}"))
}

/// What a session runs: the user's interactive shell, or (Shell ▸ New
/// Window/Tab with Same Command, `-e PROGRAM ARGS…`) one program execed
/// directly. Kept on `Session` so a later tab or window can repeat exactly
/// what this one is running, the way those two Mac Shell menu items do.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum InitialProgram {
    #[default]
    Shell,
    Exec {
        program: String,
        args: Vec<String>,
    },
}

impl From<crate::cli::ExecCommand> for InitialProgram {
    fn from(command: crate::cli::ExecCommand) -> Self {
        Self::Exec {
            program: command.program,
            args: command.args,
        }
    }
}

/// Bash does not report its current directory by default. Emit one bounded
/// OSC 7 report at each prompt without starting a process or watching /proc.
/// User prompt commands still run after the directory report.
fn bash_directory_prompt_command(shell: &str, existing: Option<String>) -> Option<String> {
    if std::path::Path::new(shell).file_name()?.to_str()? != "bash" {
        return None;
    }
    let report = r#"__rmac_uri=${PWD//%/%25}; __rmac_uri=${__rmac_uri// /%20}; __rmac_uri=${__rmac_uri//#/%23}; __rmac_uri=${__rmac_uri//\?/%3F}; printf '\033]7;file://%s\007' "$__rmac_uri""#;
    Some(
        match existing.filter(|command| !command.trim().is_empty()) {
            Some(command) => format!("{report}; {command}"),
            None => report.to_owned(),
        },
    )
}

#[cfg(test)]
mod directory_prompt_tests {
    use super::bash_directory_prompt_command;

    #[test]
    fn bash_reports_directory_at_each_prompt_and_preserves_user_hook() {
        let command =
            bash_directory_prompt_command("/bin/bash", Some("history -a".into())).unwrap();
        assert!(command.contains("7;file://%s"));
        assert!(command.ends_with("; history -a"));
        assert!(bash_directory_prompt_command("/bin/zsh", None).is_none());
    }
}

/// `portable_pty::SlavePty::spawn_command` reports every failure through
/// `anyhow`, but on Unix it's always `std::process::Command::spawn`'s own
/// `io::Error` underneath (exec failures are reported back to the parent
/// synchronously, before `spawn` returns) — never a rendered CLI message.
/// Recovering the original `io::ErrorKind` here tells "the shell doesn't
/// exist" apart from "this account can't run it" honestly, instead of one
/// generic message for both.
fn classify_shell_start_failure(error: &anyhow::Error) -> SessionStartError {
    match error
        .downcast_ref::<std::io::Error>()
        .map(std::io::Error::kind)
    {
        Some(std::io::ErrorKind::NotFound) => SessionStartError::ShellNotFound,
        Some(std::io::ErrorKind::PermissionDenied) => SessionStartError::ShellPermissionDenied,
        _ => SessionStartError::StartShell,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SessionLifecycle {
    Running,
    Exited {
        exit_code: u32,
        signal: Option<String>,
    },
    WaitFailed,
    StartFailed(SessionStartError),
}

impl SessionLifecycle {
    fn is_running(&self) -> bool {
        matches!(self, Self::Running)
    }

    fn may_be_running(&self) -> bool {
        matches!(self, Self::Running | Self::WaitFailed)
    }

    fn status_message(&self) -> Option<String> {
        match self {
            Self::Running => None,
            Self::Exited {
                exit_code: 0,
                signal: None,
            } => Some("The shell exited successfully.".into()),
            Self::Exited {
                signal: Some(signal),
                ..
            } => Some(format!("The shell was terminated by {signal}.")),
            Self::Exited { exit_code, .. } => {
                Some(format!("The shell exited with status {exit_code}."))
            }
            Self::WaitFailed => Some("Terminal could not observe the shell's exit status.".into()),
            Self::StartFailed(error) => Some(error.to_string()),
        }
    }

    fn tab_state_label(&self) -> Option<&'static str> {
        match self {
            Self::Running => None,
            Self::Exited { .. } => Some("Exited"),
            Self::WaitFailed | Self::StartFailed(_) => Some("Unavailable"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SessionStartError {
    OpenPty,
    /// The configured shell's executable does not exist (or isn't on `PATH`).
    ShellNotFound,
    /// The configured shell exists but this account can't execute it.
    ShellPermissionDenied,
    /// Any other shell-spawn failure `io::ErrorKind` doesn't distinguish.
    StartShell,
    OpenReader,
    OpenWriter,
    StartReaderWorker,
    StartWaiterWorker,
}

impl std::fmt::Display for SessionStartError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::OpenPty => "Terminal could not create a private terminal session.",
            Self::ShellNotFound => {
                "Terminal could not find the configured shell. Choose a different shell in \
                 Terminal › Settings…"
            }
            Self::ShellPermissionDenied => {
                "Terminal doesn't have permission to run the configured shell. Choose a \
                 different shell in Terminal › Settings…"
            }
            Self::StartShell => {
                "Terminal could not start the configured shell. Choose a different shell in \
                 Terminal › Settings…"
            }
            Self::OpenReader => "Terminal could not receive output from the shell.",
            Self::OpenWriter => "Terminal could not send input to the shell.",
            Self::StartReaderWorker | Self::StartWaiterWorker => {
                "Terminal could not reserve bounded resources for the configured shell."
            }
        })
    }
}

impl std::error::Error for SessionStartError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SessionControlError;

impl std::fmt::Display for SessionControlError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Terminal could not terminate the selected shell safely.")
    }
}

impl std::error::Error for SessionControlError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SessionWriteError {
    Exited,
    State,
    Write,
}

impl std::fmt::Display for SessionWriteError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Exited => "This terminal session is no longer accepting input.",
            Self::State => "Terminal could not safely access the session state.",
            Self::Write => "Terminal could not send input to the shell.",
        })
    }
}

impl std::error::Error for SessionWriteError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SessionResizeError {
    State,
    Resize,
}

impl std::fmt::Display for SessionResizeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::State => "Terminal could not safely access the session state.",
            Self::Resize => {
                "Terminal could not resize this session; it is using the last accepted size."
            }
        })
    }
}

impl std::error::Error for SessionResizeError {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct SessionTransportState {
    rejected_size: Option<TermSize>,
}

impl SessionTransportState {
    fn status_message(self, write_failed: bool) -> Option<&'static str> {
        if write_failed {
            Some("Terminal can no longer send input to this session. Existing output is readable.")
        } else if self.rejected_size.is_some() {
            Some(
                "The shell rejected the new window size. The last accepted size remains active; resize again to retry.",
            )
        } else {
            None
        }
    }
}

fn accepted_size_after_resize(
    current: TermSize,
    requested: TermSize,
    kernel_result: Result<(), SessionResizeError>,
) -> (TermSize, Result<(), SessionResizeError>) {
    match kernel_result {
        Ok(()) => (requested, Ok(())),
        Err(error) => (current, Err(error)),
    }
}

fn should_attempt_resize(
    accepted: TermSize,
    rejected: Option<TermSize>,
    requested: TermSize,
) -> bool {
    requested != accepted && rejected != Some(requested)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PasteError {
    ReviewRequired,
    UnsafeControl,
    Session(SessionWriteError),
}

impl std::fmt::Display for PasteError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::ReviewRequired => "This multiline paste requires review.",
            Self::UnsafeControl => {
                "Paste contains control characters the active program did not protect."
            }
            Self::Session(error) => return error.fmt(formatter),
        })
    }
}

impl std::error::Error for PasteError {}

fn lifecycle_after_wait(result: std::io::Result<ExitStatus>) -> SessionLifecycle {
    match result {
        Ok(status) => SessionLifecycle::Exited {
            exit_code: status.exit_code(),
            signal: status.signal().map(str::to_string),
        },
        Err(_) => SessionLifecycle::WaitFailed,
    }
}

fn foreground_job_requires_confirmation(
    running: bool,
    shell_pid: Option<u32>,
    foreground_process_group: Option<u32>,
) -> bool {
    running
        && shell_pid
            .zip(foreground_process_group)
            .is_none_or(|(shell, foreground)| shell != foreground)
}

type ReaderTask = (
    Box<dyn Read + Send>,
    Arc<Mutex<Term<EventProxy>>>,
    SessionDirectory,
    SessionShellState,
    Arc<AtomicUsize>,
    SessionJobState,
    ForegroundJobSource,
    Option<u32>,
    RedrawSender,
    // Edit ▸ Marks ▸ Automatically Mark Prompt Lines, read once at
    // `Session::spawn` like the other Settings-backed values this reader
    // thread never re-reads.
    bool,
);
type WaiterTask = (
    Box<dyn Child + Send + Sync>,
    Arc<Mutex<SessionLifecycle>>,
    RedrawSender,
);

fn retained_marker_position(
    term: &Term<EventProxy>,
    history_limit: usize,
) -> Option<MarkerPosition> {
    if history_limit == 0 || term.mode().contains(TermMode::ALT_SCREEN) {
        return None;
    }
    let grid = term.grid();
    let history_size = grid.history_size();
    if history_size >= history_limit {
        return None;
    }
    let cursor_line = usize::try_from(grid.cursor.point.line.0).ok()?;
    if cursor_line >= grid.screen_lines() {
        return None;
    }
    let (line, column) = if grid.cursor.input_needs_wrap {
        (cursor_line.saturating_add(1), 0)
    } else {
        (cursor_line, grid.cursor.point.column.0)
    };
    Some(MarkerPosition {
        line: history_size.saturating_add(line),
        column,
    })
}

fn run_reader_worker(
    (
        mut reader,
        term,
        directory,
        shell,
        scrollback_limit,
        job_state,
        job_source,
        shell_pid,
        redraw,
        auto_mark_prompts,
    ): ReaderTask,
) {
    let mut parser: Processor = Processor::new();
    let mut output_filter = OutputFilter::default();
    let mut buffer = [0u8; 8192];
    let mut filtered = Vec::with_capacity(buffer.len());
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                output_filter.filter_into(&buffer[..read], &mut filtered);
                if let Some(uri) = output_filter.take_current_directory_uri() {
                    directory.set_uri(&uri);
                }
                let markers = output_filter.take_shell_markers();
                job_state.refresh(&job_source, shell_pid);
                if filtered.is_empty() {
                    continue;
                }
                if let Ok(mut term) = term.lock() {
                    let mut start = 0;
                    for marker in markers {
                        let end = marker.output_offset.clamp(start, filtered.len());
                        advance_filtered_output(&mut parser, &mut *term, &filtered[start..end]);
                        let position = retained_marker_position(
                            &term,
                            scrollback_limit.load(Ordering::Acquire),
                        );
                        shell.set_marker(&marker.payload, position, auto_mark_prompts);
                        start = end;
                    }
                    advance_filtered_output(&mut parser, &mut *term, &filtered[start..]);
                } else {
                    for marker in markers {
                        shell.set_marker(&marker.payload, None, auto_mark_prompts);
                    }
                }
                request_redraw(&redraw);
            }
        }
    }
    request_redraw(&redraw);
}

fn run_waiter_worker((mut child, lifecycle, redraw): WaiterTask) {
    let next = lifecycle_after_wait(child.wait());
    if let Ok(mut lifecycle) = lifecycle.lock() {
        *lifecycle = next;
    }
    request_redraw(&redraw);
}

fn reserve_session_worker<Task, Run>(
    name: &'static str,
    run: Run,
) -> std::io::Result<(SyncSender<Task>, JoinHandle<()>)>
where
    Task: Send + 'static,
    Run: FnOnce(Task) + Send + 'static,
{
    let (sender, receiver) = sync_channel(0);
    let handle = std::thread::Builder::new()
        .name(name.into())
        .stack_size(SESSION_WORKER_STACK_BYTES)
        .spawn(move || {
            if let Ok(task) = receiver.recv() {
                run(task);
            }
        })?;
    Ok((sender, handle))
}

struct ReservedSessionWorkers {
    reader_sender: SyncSender<ReaderTask>,
    reader_handle: JoinHandle<()>,
    waiter_sender: SyncSender<WaiterTask>,
    waiter_handle: JoinHandle<()>,
}

impl ReservedSessionWorkers {
    fn reserve() -> Result<Self, SessionStartError> {
        let (reader_sender, reader_handle) =
            reserve_session_worker("rmac-terminal-reader", run_reader_worker)
                .map_err(|_| SessionStartError::StartReaderWorker)?;
        let (waiter_sender, waiter_handle) =
            match reserve_session_worker("rmac-terminal-waiter", run_waiter_worker) {
                Ok(worker) => worker,
                Err(_) => {
                    drop(reader_sender);
                    let _ = reader_handle.join();
                    return Err(SessionStartError::StartWaiterWorker);
                }
            };
        Ok(Self {
            reader_sender,
            reader_handle,
            waiter_sender,
            waiter_handle,
        })
    }

    fn activate(
        self,
        reader_task: ReaderTask,
        waiter_task: WaiterTask,
        killer: &mut dyn ChildKiller,
    ) -> Result<(), SessionStartError> {
        let Self {
            reader_sender,
            reader_handle,
            waiter_sender,
            waiter_handle,
        } = self;

        if let Err(error) = waiter_sender.send(waiter_task) {
            let (mut child, _, _) = error.0;
            let _ = child.kill();
            let _ = child.wait();
            drop(reader_sender);
            let _ = reader_handle.join();
            let _ = waiter_handle.join();
            return Err(SessionStartError::StartWaiterWorker);
        }

        if reader_sender.send(reader_task).is_err() {
            let _ = killer.kill();
            let _ = reader_handle.join();
            drop(waiter_handle);
            return Err(SessionStartError::StartReaderWorker);
        }

        drop(reader_handle);
        drop(waiter_handle);
        Ok(())
    }
}

pub(super) struct Session {
    pub(super) id: u64,
    pub(super) term: Arc<Mutex<Term<EventProxy>>>,
    pub(super) ui: SessionUiState,
    pub(super) accepted_size: TermSize,
    transport: SessionTransportState,
    writer: SharedWriter,
    write_failed: Arc<AtomicBool>,
    title: SessionTitle,
    /// Shell ▸ Edit Title (⇧⌘I): a user-set override, separate from the
    /// automatic OSC/job/directory title above it so a later shell title
    /// escape never silently replaces what the user typed.
    manual_title: SessionTitle,
    /// Terminal's own default title — "<user> — -<shell>" — used below the
    /// job/OSC title and above the working-directory fallback (UIA-02). Only
    /// an interactive-shell session has one; a session execed directly
    /// (`-e PROGRAM`) falls straight through to the directory label, as
    /// before, once its foreground job finishes.
    default_identity: Option<String>,
    directory: SessionDirectory,
    shell_state: SessionShellState,
    scrollback_limit: Arc<AtomicUsize>,
    job_state: SessionJobState,
    master: Option<Box<dyn MasterPty + Send>>,
    shell_pid: Option<u32>,
    killer: Option<Box<dyn ChildKiller + Send + Sync>>,
    lifecycle: Arc<Mutex<SessionLifecycle>>,
    origin: InitialProgram,
}

impl Session {
    pub(super) fn spawn(
        cols: usize,
        rows: usize,
        scrollback_lines: usize,
        starting_directory: Option<PathBuf>,
        redraw: RedrawSender,
        program: InitialProgram,
    ) -> Result<Self, SessionStartError> {
        let size = TermSize { cols, lines: rows };
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows: rows as u16,
                cols: cols as u16,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|_| SessionStartError::OpenPty)?;
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|_| SessionStartError::OpenReader)?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|_| SessionStartError::OpenWriter)?;
        let writer: SharedWriter = Arc::new(Mutex::new(writer));
        let write_failed = Arc::new(AtomicBool::new(false));
        let title = SessionTitle::default();
        let starting_directory = starting_directory.or_else(|| std::env::current_dir().ok());
        let directory = starting_directory
            .as_deref()
            .map(SessionDirectory::from_local)
            .unwrap_or_default();
        let shell_state = SessionShellState::default();
        let scrollback_limit = Arc::new(AtomicUsize::new(scrollback_lines));
        let job_state = SessionJobState::default();
        let job_source = ForegroundJobSource::from_master(&*pair.master);
        let workers = ReservedSessionWorkers::reserve()?;
        let mut default_identity = None;
        let mut command = match &program {
            InitialProgram::Shell => {
                let shell = shell_program(std::env::var("SHELL").ok());
                let user = std::env::var("USER")
                    .or_else(|_| std::env::var("LOGNAME"))
                    .ok();
                default_identity = default_shell_identity(user.as_deref(), &shell);
                let prompt_command =
                    bash_directory_prompt_command(&shell, std::env::var("PROMPT_COMMAND").ok());
                let mut command = CommandBuilder::new(shell);
                if let Some(prompt_command) = prompt_command {
                    command.env("PROMPT_COMMAND", prompt_command);
                }
                command
            }
            // `-e PROGRAM ARGS…`, or Shell ▸ New Window/Tab with Same
            // Command repeating one: execed directly, argv untouched —
            // never through a shell, so nothing here re-splits or
            // re-expands ARGS.
            InitialProgram::Exec { program, args } => {
                let mut command = CommandBuilder::new(program);
                command.args(args);
                command
            }
        };
        command.env("TERM", "xterm-256color");
        if let Some(directory) = starting_directory {
            command.cwd(directory);
        }
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| classify_shell_start_failure(&error))?;
        let shell_pid = child.process_id();
        let mut killer = child.clone_killer();
        drop(pair.slave);
        let term = Arc::new(Mutex::new(Term::new(
            terminal_config(scrollback_lines),
            &size,
            EventProxy::connected(
                Arc::clone(&writer),
                Arc::clone(&write_failed),
                title.clone(),
                redraw.clone(),
            ),
        )));
        let lifecycle = Arc::new(Mutex::new(SessionLifecycle::Running));
        let auto_mark_prompts = crate::profiles::load_automatically_mark_prompt_lines();
        workers.activate(
            (
                reader,
                Arc::clone(&term),
                directory.clone(),
                shell_state.clone(),
                Arc::clone(&scrollback_limit),
                job_state.clone(),
                job_source,
                shell_pid,
                redraw.clone(),
                auto_mark_prompts,
            ),
            (child, Arc::clone(&lifecycle), redraw),
            killer.as_mut(),
        )?;
        Ok(Self {
            id: NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed),
            term,
            ui: SessionUiState::default(),
            accepted_size: size,
            transport: SessionTransportState::default(),
            writer,
            write_failed,
            title,
            manual_title: SessionTitle::default(),
            default_identity,
            directory,
            shell_state,
            scrollback_limit,
            job_state,
            master: Some(pair.master),
            shell_pid,
            killer: Some(killer),
            lifecycle,
            origin: program,
        })
    }

    pub(super) fn failed(
        cols: usize,
        rows: usize,
        scrollback_lines: usize,
        error: SessionStartError,
    ) -> Self {
        let size = TermSize { cols, lines: rows };
        let term = Arc::new(Mutex::new(Term::new(
            terminal_config(scrollback_lines),
            &size,
            EventProxy::default(),
        )));
        if let Ok(mut term) = term.lock() {
            let mut parser: Processor = Processor::new();
            let text = format!("\r\n  {error}\r\n");
            parser.advance(&mut *term, text.as_bytes());
        }
        Self {
            id: NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed),
            term,
            ui: SessionUiState::default(),
            accepted_size: size,
            transport: SessionTransportState::default(),
            writer: Arc::new(Mutex::new(
                Box::new(std::io::sink()) as Box<dyn Write + Send>
            )),
            write_failed: Arc::new(AtomicBool::new(false)),
            title: SessionTitle::default(),
            manual_title: SessionTitle::default(),
            default_identity: None,
            directory: SessionDirectory::default(),
            shell_state: SessionShellState::default(),
            scrollback_limit: Arc::new(AtomicUsize::new(scrollback_lines)),
            job_state: SessionJobState::default(),
            master: None,
            shell_pid: None,
            killer: None,
            lifecycle: Arc::new(Mutex::new(SessionLifecycle::StartFailed(error))),
            origin: InitialProgram::default(),
        }
    }

    /// Shell ▸ New Window/Tab with Same Command: `Some` only for a session
    /// that was itself execed directly (`-e`, or a same-command relaunch of
    /// one), never for an ordinary interactive shell.
    pub(super) fn exec_origin(&self) -> Option<crate::cli::ExecCommand> {
        match &self.origin {
            InitialProgram::Shell => None,
            InitialProgram::Exec { program, args } => Some(crate::cli::ExecCommand {
                program: program.clone(),
                args: args.clone(),
            }),
        }
    }

    fn lifecycle(&self) -> SessionLifecycle {
        self.lifecycle
            .lock()
            .map(|lifecycle| lifecycle.clone())
            .unwrap_or(SessionLifecycle::WaitFailed)
    }

    pub(super) fn accepts_input(&self) -> bool {
        self.lifecycle().is_running() && !self.write_failed.load(Ordering::Acquire)
    }

    /// Settings ▸ Shell ▸ "Close if the shell exited cleanly": true only
    /// once the shell itself has exited with status 0 and no signal — never
    /// true for a still-running shell, a non-zero exit, a signal, or a
    /// `WaitFailed`/`StartFailed` session, which keep today's "stay open and
    /// show a status message" behaviour regardless of the setting.
    pub(super) fn exited_cleanly(&self) -> bool {
        matches!(
            self.lifecycle(),
            SessionLifecycle::Exited {
                exit_code: 0,
                signal: None,
            }
        )
    }

    pub(super) fn tab_state_label(&self) -> Option<String> {
        if self.lifecycle().is_running() && self.write_failed.load(Ordering::Acquire) {
            Some("Unavailable".into())
        } else {
            self.lifecycle()
                .tab_state_label()
                .map(str::to_owned)
                .or_else(|| self.shell_state.tab_label())
        }
    }

    pub(super) fn tab_title(&self) -> Option<String> {
        // Shell ▸ Edit Title (⇧⌘I): a title the user typed outranks the
        // automatic ones below, exactly as it does on the Mac — it keeps
        // showing even while a program is in the foreground, until the
        // user edits or clears it again.
        if let Some(manual) = self.manual_title.current() {
            return Some(manual);
        }
        let job = if self.lifecycle().is_running() {
            self.job_state.label()
        } else {
            None
        };
        job.or_else(|| self.title.current())
            // A directory the shell has actually reported over OSC 7 (a
            // real `cd`) outranks the static default identity below, but
            // the unconfirmed starting guess `from_local` set before the
            // shell said anything does not — that guess is exactly the
            // "repo" directory-name title UIA-02 found Lulo showing by
            // default instead of the Mac's "user — shell".
            .or_else(|| self.directory.confirmed_label())
            .or_else(|| self.default_identity.clone())
            .or_else(|| self.directory.label())
    }

    /// Shell ▸ Edit Title (⇧⌘I): `None` (an empty field) clears the
    /// override and returns to the automatic title above.
    pub(super) fn set_manual_title(&self, title: Option<&str>) -> bool {
        self.manual_title.set(title)
    }

    pub(super) fn working_directory(&self) -> Option<PathBuf> {
        self.directory.live_local_path()
    }

    /// Application ▸ Quit and Keep Windows (TERM-22): this tab's whole
    /// buffer (scrollback and screen), to capture before the window
    /// closes. Like `TerminalView::buffer_text` (Shell ▸ Export Text
    /// As…/Print…) but callable for any tab, not just the active one.
    pub(super) fn buffer_text(&self, rows: usize, cols: usize) -> Option<String> {
        let term = self.term.lock().ok()?;
        Some(crate::ui_state::buffer_text(&term, rows, cols))
    }

    /// Application ▸ Quit and Keep Windows (TERM-22): print `text` — a
    /// bounded, plain-text snapshot of the kept tab's own buffer, never
    /// raw escape sequences — straight into the grid before anything else
    /// writes to it, so a restored tab's scrollback starts with what was
    /// there last time. This is the same `Processor::advance` path the
    /// reader thread itself uses for real PTY bytes (see `Session::failed`
    /// for the same trick with a startup-error message); it is not
    /// reconnected to any process, just printed.
    pub(super) fn inject_restored_scrollback(&self, text: &str) {
        if text.is_empty() {
            return;
        }
        let Ok(mut term) = self.term.lock() else {
            return;
        };
        let mut parser: Processor = Processor::new();
        // Each captured line started at column 0; a lone `\n` would only
        // move down a row without returning there, so every line break
        // gets its own carriage return.
        let replayed = text.replace('\n', "\r\n");
        parser.advance(&mut *term, replayed.as_bytes());
        parser.advance(&mut *term, b"\r\n");
    }

    pub(super) fn status_message(&self) -> Option<String> {
        self.lifecycle().status_message().or_else(|| {
            self.transport
                .status_message(self.write_failed.load(Ordering::Acquire))
                .map(str::to_string)
        })
    }

    pub(super) fn resize(&mut self, requested: TermSize) -> Result<(), SessionResizeError> {
        if requested == self.accepted_size {
            self.transport.rejected_size = None;
            return Ok(());
        }
        if !should_attempt_resize(self.accepted_size, self.transport.rejected_size, requested) {
            return Err(SessionResizeError::Resize);
        }
        let Ok(mut term) = self.term.lock() else {
            self.transport.rejected_size = Some(requested);
            return Err(SessionResizeError::State);
        };
        let kernel_result = self.master.as_ref().map_or(Ok(()), |master| {
            master
                .resize(PtySize {
                    rows: requested.lines as u16,
                    cols: requested.cols as u16,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|_| SessionResizeError::Resize)
        });
        let (accepted, result) =
            accepted_size_after_resize(self.accepted_size, requested, kernel_result);
        if result.is_err() {
            self.transport.rejected_size = Some(requested);
            return result;
        }
        term.resize(accepted);
        self.accepted_size = accepted;
        self.transport.rejected_size = None;
        self.shell_state.clear_grid_marks();
        Ok(())
    }

    pub(super) fn set_scrollback_limit(&self, limit: usize) {
        self.scrollback_limit.store(limit, Ordering::Release);
        self.shell_state.clear_grid_marks();
    }

    pub(super) fn clear_shell_marks(&self) {
        self.shell_state.clear_grid_marks();
    }

    pub(super) fn mark_current_line(&self, bookmark: bool) -> bool {
        let Ok(term) = self.term.lock() else {
            return false;
        };
        let Some(position) =
            retained_marker_position(&term, self.scrollback_limit.load(Ordering::Acquire))
        else {
            return false;
        };
        self.shell_state.mark_line(position.line, bookmark);
        true
    }

    pub(super) fn can_mark_current_line(&self) -> bool {
        self.term.lock().is_ok_and(|term| {
            retained_marker_position(&term, self.scrollback_limit.load(Ordering::Acquire)).is_some()
        })
    }

    pub(super) fn unmark_current_line(&self) -> bool {
        let Ok(term) = self.term.lock() else {
            return false;
        };
        let Some(position) =
            retained_marker_position(&term, self.scrollback_limit.load(Ordering::Acquire))
        else {
            return false;
        };
        self.shell_state.unmark_line(position.line);
        true
    }

    pub(super) fn can_scroll_to_bookmark(&self, direction: PromptDirection) -> bool {
        if !self.shell_state.has_bookmarks() {
            return false;
        }
        let Ok(term) = self.term.lock() else {
            return false;
        };
        if term.mode().contains(TermMode::ALT_SCREEN) {
            return false;
        }
        let grid = term.grid();
        self.shell_state
            .bookmark_offset(
                direction,
                grid.history_size(),
                grid.display_offset(),
                self.scrollback_limit.load(Ordering::Acquire),
            )
            .is_some_and(|offset| offset != grid.display_offset())
    }

    pub(super) fn current_line_is_marked(&self) -> bool {
        let Ok(term) = self.term.lock() else {
            return false;
        };
        retained_marker_position(&term, self.scrollback_limit.load(Ordering::Acquire))
            .is_some_and(|position| self.shell_state.has_mark_at(position.line))
    }

    pub(super) fn can_select_to_mark(
        &self,
        direction: PromptDirection,
        bookmark_only: bool,
    ) -> bool {
        let Ok(term) = self.term.lock() else {
            return false;
        };
        let limit = self.scrollback_limit.load(Ordering::Acquire);
        let Some(position) = retained_marker_position(&term, limit) else {
            return false;
        };
        self.shell_state
            .selection_mark(
                direction,
                bookmark_only,
                position.line,
                term.grid().history_size(),
                limit,
            )
            .is_some()
    }

    pub(super) fn select_to_mark(
        &mut self,
        direction: PromptDirection,
        bookmark_only: bool,
    ) -> Result<bool, SessionWriteError> {
        let mut term = self.term.lock().map_err(|_| SessionWriteError::State)?;
        let limit = self.scrollback_limit.load(Ordering::Acquire);
        let Some(position) = retained_marker_position(&term, limit) else {
            return Ok(false);
        };
        let history_size = term.grid().history_size();
        let Some(target) = self.shell_state.selection_mark(
            direction,
            bookmark_only,
            position.line,
            history_size,
            limit,
        ) else {
            return Ok(false);
        };
        let history_line = i32::try_from(history_size).unwrap_or(i32::MAX);
        let target_line = i32::try_from(target)
            .unwrap_or(i32::MAX)
            .saturating_sub(history_line);
        let cursor_line = i32::try_from(position.line)
            .unwrap_or(i32::MAX)
            .saturating_sub(history_line);
        self.ui.selection = Some(Selection {
            anchor: (cursor_line, position.column),
            head: (target_line, 0),
        });
        let offset = history_size.saturating_sub(target);
        term.scroll_display(Scroll::Bottom);
        term.scroll_display(Scroll::Delta(i32::try_from(offset).unwrap_or(i32::MAX)));
        Ok(true)
    }

    pub(super) fn can_clear_to_mark(&self, bookmark_only: bool) -> bool {
        let Ok(term) = self.term.lock() else {
            return false;
        };
        let grid = term.grid();
        self.shell_state
            .clear_to_mark_line(
                bookmark_only,
                grid.history_size(),
                grid.display_offset(),
                self.scrollback_limit.load(Ordering::Acquire),
            )
            .is_some()
    }

    /// Edit ▸ Clear to Previous Mark (⌘L) / Clear to Previous Bookmark
    /// (⌥⌘L): drop every retained scrollback row older than the nearest
    /// mark/bookmark above the viewport, keeping that row and everything
    /// after it. The configured scrollback limit (`terminal_config`'s
    /// `max_scrollback`) is restored right after, so this only ever
    /// shortens *today's* history, never the budget future output can
    /// refill — the same two-step `update_history` dance `set_scrollback_limit`
    /// uses for the cross-tab budget. Marks are coordinates into a grid that
    /// just got shorter, so — like every other operation that structurally
    /// changes the grid — they're all cleared afterward rather than risking
    /// a stale one pointing at the wrong row.
    pub(super) fn clear_to_previous_mark(
        &mut self,
        bookmark_only: bool,
    ) -> Result<bool, SessionWriteError> {
        let mut term = self.term.lock().map_err(|_| SessionWriteError::State)?;
        let limit = self.scrollback_limit.load(Ordering::Acquire);
        let (history_size, display_offset) = {
            let grid = term.grid();
            (grid.history_size(), grid.display_offset())
        };
        let Some(target_line) =
            self.shell_state
                .clear_to_mark_line(bookmark_only, history_size, display_offset, limit)
        else {
            return Ok(false);
        };
        let keep = history_size.saturating_sub(target_line);
        term.grid_mut().update_history(keep);
        term.grid_mut().update_history(limit);
        drop(term);
        self.shell_state.clear_grid_marks();
        Ok(true)
    }

    pub(super) fn scroll_to_bookmark(
        &self,
        direction: PromptDirection,
    ) -> Result<bool, SessionWriteError> {
        let mut term = self.term.lock().map_err(|_| SessionWriteError::State)?;
        if term.mode().contains(TermMode::ALT_SCREEN) {
            return Ok(false);
        }
        let grid = term.grid();
        let Some(offset) = self.shell_state.bookmark_offset(
            direction,
            grid.history_size(),
            grid.display_offset(),
            self.scrollback_limit.load(Ordering::Acquire),
        ) else {
            return Ok(false);
        };
        if offset == grid.display_offset() {
            return Ok(false);
        }
        term.scroll_display(Scroll::Bottom);
        term.scroll_display(Scroll::Delta(i32::try_from(offset).unwrap_or(i32::MAX)));
        Ok(true)
    }

    /// View ▸ Show Marks (TERM-23): whether this tab has anything a
    /// gutter indicator could show.
    pub(super) fn has_any_marks(&self) -> bool {
        self.shell_state.has_any_marks()
    }

    /// View ▸ Show Marks (TERM-23): whether the given absolute line
    /// (`history_size`-shifted, like `retained_marker_position`'s own
    /// coordinates) is marked or bookmarked.
    pub(super) fn has_mark_at_absolute_line(&self, line: usize) -> bool {
        self.shell_state.has_mark_at(line)
    }

    /// Edit ▸ Bookmarks ▸: every bookmarked line in this tab, oldest first.
    pub(super) fn bookmark_lines(&self) -> Vec<usize> {
        let Ok(term) = self.term.lock() else {
            return Vec::new();
        };
        self.shell_state.bookmark_lines(
            term.grid().history_size(),
            self.scrollback_limit.load(Ordering::Acquire),
        )
    }

    /// Edit ▸ Bookmarks ▸ <a listed bookmark>: scroll straight to it,
    /// unlike `scroll_to_bookmark`'s Previous/Next, which moves relative
    /// to the viewport.
    pub(super) fn scroll_to_bookmark_line(&self, line: usize) -> Result<bool, SessionWriteError> {
        let mut term = self.term.lock().map_err(|_| SessionWriteError::State)?;
        if term.mode().contains(TermMode::ALT_SCREEN) {
            return Ok(false);
        }
        let grid = term.grid();
        let offset = grid.history_size().saturating_sub(line);
        if offset == grid.display_offset() {
            return Ok(false);
        }
        term.scroll_display(Scroll::Bottom);
        term.scroll_display(Scroll::Delta(i32::try_from(offset).unwrap_or(i32::MAX)));
        Ok(true)
    }

    pub(super) fn scroll_to_prompt(
        &self,
        direction: PromptDirection,
    ) -> Result<bool, SessionWriteError> {
        let mut term = self.term.lock().map_err(|_| SessionWriteError::State)?;
        let grid = term.grid();
        let Some(offset) = self.shell_state.prompt_offset(
            direction,
            grid.history_size(),
            grid.display_offset(),
            self.scrollback_limit.load(Ordering::Acquire),
        ) else {
            return Ok(false);
        };
        if offset == grid.display_offset() {
            return Ok(false);
        }
        term.scroll_display(Scroll::Bottom);
        term.scroll_display(Scroll::Delta(i32::try_from(offset).unwrap_or(i32::MAX)));
        Ok(true)
    }

    pub(super) fn select_command_range(
        &mut self,
        kind: CommandRangeKind,
    ) -> Result<bool, SessionWriteError> {
        let mut term = self.term.lock().map_err(|_| SessionWriteError::State)?;
        let grid = term.grid();
        let history_size = grid.history_size();
        let Some(range) = self.shell_state.command_range(
            kind,
            history_size,
            grid.display_offset(),
            self.scrollback_limit.load(Ordering::Acquire),
            grid.columns(),
        ) else {
            return Ok(false);
        };
        let history_line = i32::try_from(history_size).unwrap_or(i32::MAX);
        let to_grid = |position: MarkerPosition| {
            (
                i32::try_from(position.line)
                    .unwrap_or(i32::MAX)
                    .saturating_sub(history_line),
                position.column,
            )
        };
        let selection = Selection {
            anchor: to_grid(range.start),
            head: to_grid(range.end),
        };
        let offset = history_size.saturating_sub(range.start.line);
        term.scroll_display(Scroll::Bottom);
        term.scroll_display(Scroll::Delta(i32::try_from(offset).unwrap_or(i32::MAX)));
        drop(term);
        self.ui.selection = Some(selection);
        Ok(true)
    }

    /// The name of the program running in the foreground, when there is one
    /// (a job, not the idle shell) — what Terminal lists before it closes.
    pub(super) fn foreground_job_name(&self) -> Option<String> {
        if !self.has_foreground_job() {
            return None;
        }
        self.job_state.label()
    }

    pub(super) fn has_foreground_job(&self) -> bool {
        let may_be_running = self.lifecycle().may_be_running();
        #[cfg(unix)]
        let foreground_process_group = self
            .master
            .as_ref()
            .and_then(|master| master.process_group_leader())
            .and_then(|pid| u32::try_from(pid).ok());
        #[cfg(not(unix))]
        let foreground_process_group = None;
        foreground_job_requires_confirmation(
            may_be_running,
            self.shell_pid,
            foreground_process_group,
        )
    }

    pub(super) fn terminate(&mut self) -> Result<(), SessionControlError> {
        if !self.lifecycle().may_be_running() {
            return Ok(());
        }
        #[cfg(unix)]
        if let Some(process_group) = self
            .master
            .as_ref()
            .and_then(|master| master.process_group_leader())
            .filter(|process_group| *process_group > 0)
            .filter(|process_group| u32::try_from(*process_group).ok() != self.shell_pid)
        {
            // SAFETY: this is the positive foreground group reported by the
            // still-owned PTY; negation addresses that exact process group.
            let result = unsafe { libc::kill(-process_group, libc::SIGHUP) };
            if result != 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH) {
                return Err(SessionControlError);
            }
        }
        self.killer
            .as_mut()
            .ok_or(SessionControlError)?
            .kill()
            .map_err(|_| SessionControlError)
    }

    pub(super) fn write(&mut self, bytes: &[u8]) -> Result<(), SessionWriteError> {
        if !self.lifecycle().is_running() {
            return Err(SessionWriteError::Exited);
        }
        if self.write_failed.load(Ordering::Acquire) {
            return Err(SessionWriteError::Write);
        }
        let result = self
            .writer
            .lock()
            .map_err(|_| SessionWriteError::Write)
            .and_then(|mut writer| {
                writer
                    .write_all(bytes)
                    .and_then(|_| writer.flush())
                    .map_err(|_| SessionWriteError::Write)
            });
        if result.is_err() {
            self.write_failed.store(true, Ordering::Release);
        }
        result
    }

    pub(super) fn paste(&mut self, text: &str, reviewed_multiline: bool) -> Result<(), PasteError> {
        if !self.lifecycle().is_running() {
            return Err(PasteError::Session(SessionWriteError::Exited));
        }
        let term = self
            .term
            .lock()
            .map_err(|_| PasteError::Session(SessionWriteError::State))?;
        let bracketed = term.mode().contains(TermMode::BRACKETED_PASTE);
        if !bracketed {
            if has_unsafe_unbracketed_control(text) {
                return Err(PasteError::UnsafeControl);
            }
            if logical_line_count(text) > 1 && !reviewed_multiline {
                return Err(PasteError::ReviewRequired);
            }
        }
        drop(term);
        self.write(&prepare_paste(text, bracketed))
            .map_err(PasteError::Session)?;
        if let Ok(mut term) = self.term.lock() {
            term.scroll_display(alacritty_terminal::grid::Scroll::Bottom);
        }
        Ok(())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.lifecycle().may_be_running() {
            let _ = self.killer.as_mut().map(|killer| killer.kill());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::MAX_TABS;
    use crate::emulator::SCROLLBACK_LINES;

    struct FailingWriter;

    impl Write for FailingWriter {
        fn write(&mut self, _buffer: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "private writer diagnostic",
            ))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct RecordingWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for RecordingWriter {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn parser_protocol_replies_share_the_session_writer() {
        let output = Arc::new(Mutex::new(Vec::new()));
        let writer: SharedWriter =
            Arc::new(Mutex::new(Box::new(RecordingWriter(Arc::clone(&output)))));
        let write_failed = Arc::new(AtomicBool::new(false));
        let title = SessionTitle::default();
        let (redraw, redraw_rx) = async_channel::bounded(1);
        let proxy = EventProxy::connected(writer, Arc::clone(&write_failed), title.clone(), redraw);
        let size = TermSize { cols: 20, lines: 5 };
        let mut term = Term::new(terminal_config(10), &size, proxy.clone());
        let mut parser: Processor = Processor::new();

        parser.advance(&mut term, b"\x1b[?u");
        assert_eq!(&*output.lock().unwrap(), b"\x1b[?0u");
        parser.advance(&mut term, b"\x1b[>5u\x1b[?u");
        assert_eq!(
            &*output.lock().unwrap(),
            b"\x1b[?0u\x1b[?5u",
            "mode query replies must reach the exact PTY writer"
        );
        assert!(!write_failed.load(Ordering::Acquire));

        parser.advance(&mut term, b"\x1b]0;  vim   workspace  \x07");
        assert_eq!(title.current().as_deref(), Some("vim workspace"));
        assert_eq!(redraw_rx.try_recv(), Ok(()));
        proxy.send_event(Event::Title("vim workspace".into()));
        assert!(
            redraw_rx.try_recv().is_err(),
            "an unchanged title stays idle"
        );
        proxy.send_event(Event::ResetTitle);
        assert_eq!(title.current(), None);
        assert_eq!(redraw_rx.try_recv(), Ok(()));
    }

    #[test]
    fn redraw_requests_coalesce_until_the_ui_consumes_one() {
        let (sender, receiver) = async_channel::bounded(1);
        request_redraw(&sender);
        request_redraw(&sender);
        request_redraw(&sender);
        assert_eq!(receiver.len(), 1);
        assert_eq!(receiver.try_recv(), Ok(()));
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn shell_fallback_is_portable_and_ignores_empty_configuration() {
        assert_eq!(shell_program(None), default_shell());
        assert_eq!(shell_program(Some("  ".to_string())), default_shell());
        assert_eq!(shell_program(Some("/bin/fish".to_string())), "/bin/fish");
    }

    /// UIA-02: a plain shell window's default title is "<user> — -<shell>",
    /// matching the Mac's "jake — -zsh" — never blank just because no job
    /// is running yet and no OSC title has arrived.
    #[test]
    fn default_shell_identity_matches_the_macs_user_dash_shell_format() {
        assert_eq!(
            default_shell_identity(Some("jake"), "/bin/zsh"),
            Some("jake — -zsh".to_string())
        );
        assert_eq!(
            default_shell_identity(Some("jake"), "/usr/bin/fish"),
            Some("jake — -fish".to_string())
        );
        assert_eq!(default_shell_identity(None, "/bin/zsh"), None);
        assert_eq!(default_shell_identity(Some("  "), "/bin/zsh"), None);
    }

    #[test]
    fn foreground_job_close_review_fails_closed() {
        assert!(!foreground_job_requires_confirmation(
            false,
            Some(40),
            Some(41)
        ));
        assert!(!foreground_job_requires_confirmation(
            true,
            Some(40),
            Some(40)
        ));
        assert!(foreground_job_requires_confirmation(
            true,
            Some(40),
            Some(41)
        ));
        assert!(foreground_job_requires_confirmation(true, None, Some(41)));
        assert!(foreground_job_requires_confirmation(true, Some(40), None));
    }

    #[test]
    fn child_exit_states_are_truthful_and_private_safe() {
        assert_eq!(SessionLifecycle::Running.status_message(), None);
        assert!(SessionLifecycle::WaitFailed.may_be_running());
        assert!(!SessionLifecycle::StartFailed(SessionStartError::StartShell).may_be_running());
        assert_eq!(
            SessionLifecycle::Exited {
                exit_code: 0,
                signal: None
            }
            .status_message()
            .as_deref(),
            Some("The shell exited successfully.")
        );
        assert_eq!(
            SessionLifecycle::Exited {
                exit_code: 7,
                signal: None
            }
            .status_message()
            .as_deref(),
            Some("The shell exited with status 7.")
        );
        assert_eq!(
            SessionLifecycle::Exited {
                exit_code: 1,
                signal: Some("Hangup".into())
            }
            .status_message()
            .as_deref(),
            Some("The shell was terminated by Hangup.")
        );
        assert_eq!(
            lifecycle_after_wait(Ok(ExitStatus::with_exit_code(9))),
            SessionLifecycle::Exited {
                exit_code: 9,
                signal: None
            }
        );
        assert_eq!(
            lifecycle_after_wait(Ok(ExitStatus::with_signal("Hangup"))),
            SessionLifecycle::Exited {
                exit_code: 1,
                signal: Some("Hangup".into())
            }
        );
        assert_eq!(
            lifecycle_after_wait(Err(std::io::Error::other("private diagnostic"))),
            SessionLifecycle::WaitFailed
        );
    }

    #[test]
    fn shell_start_failure_is_classified_by_io_error_kind() {
        let not_found = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::NotFound));
        assert_eq!(
            classify_shell_start_failure(&not_found),
            SessionStartError::ShellNotFound
        );

        let denied = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        assert_eq!(
            classify_shell_start_failure(&denied),
            SessionStartError::ShellPermissionDenied
        );

        // Any other io::ErrorKind falls back to the generic message rather
        // than a false "not found" or "permission denied" claim.
        let other = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::Other));
        assert_eq!(
            classify_shell_start_failure(&other),
            SessionStartError::StartShell
        );

        // A failure that isn't an io::Error at all (e.g. from a non-Unix
        // portable_pty backend) also falls back, instead of panicking.
        let not_io = anyhow::anyhow!("private diagnostic");
        assert_eq!(
            classify_shell_start_failure(&not_io),
            SessionStartError::StartShell
        );
    }

    #[test]
    fn shell_start_error_messages_name_the_cause_and_a_recovery_path() {
        assert_eq!(
            SessionStartError::ShellNotFound.to_string(),
            "Terminal could not find the configured shell. Choose a different shell in \
             Terminal › Settings…"
        );
        assert_eq!(
            SessionStartError::ShellPermissionDenied.to_string(),
            "Terminal doesn't have permission to run the configured shell. Choose a different \
             shell in Terminal › Settings…"
        );
        assert_eq!(
            SessionLifecycle::StartFailed(SessionStartError::ShellNotFound).status_message(),
            Some(SessionStartError::ShellNotFound.to_string())
        );
    }

    #[test]
    fn resize_failure_retains_the_last_kernel_accepted_geometry() {
        let current = TermSize {
            cols: 100,
            lines: 28,
        };
        let requested = TermSize {
            cols: 140,
            lines: 42,
        };
        let (accepted, result) =
            accepted_size_after_resize(current, requested, Err(SessionResizeError::Resize));
        assert_eq!(accepted, current);
        assert_eq!(result, Err(SessionResizeError::Resize));
        let (accepted, result) = accepted_size_after_resize(current, requested, Ok(()));
        assert_eq!(accepted, requested);
        assert_eq!(result, Ok(()));
        assert!(should_attempt_resize(current, None, requested));
        assert!(!should_attempt_resize(current, Some(requested), requested));
        assert!(should_attempt_resize(
            current,
            Some(requested),
            TermSize {
                cols: 141,
                lines: 42,
            }
        ));
    }

    #[test]
    fn writer_failure_permanently_disables_misleading_live_input() {
        let size = TermSize { cols: 20, lines: 5 };
        let mut session = Session {
            id: 1,
            term: Arc::new(Mutex::new(Term::new(
                terminal_config(10),
                &size,
                EventProxy::default(),
            ))),
            ui: SessionUiState::default(),
            accepted_size: size,
            transport: SessionTransportState::default(),
            writer: Arc::new(Mutex::new(Box::new(FailingWriter))),
            write_failed: Arc::new(AtomicBool::new(false)),
            title: SessionTitle::default(),
            manual_title: SessionTitle::default(),
            default_identity: None,
            directory: SessionDirectory::default(),
            shell_state: SessionShellState::default(),
            scrollback_limit: Arc::new(AtomicUsize::new(10)),
            job_state: SessionJobState::default(),
            master: None,
            shell_pid: None,
            killer: None,
            lifecycle: Arc::new(Mutex::new(SessionLifecycle::Running)),
            origin: InitialProgram::default(),
        };

        assert!(session.accepts_input());
        assert_eq!(session.write(b"private"), Err(SessionWriteError::Write));
        assert!(!session.accepts_input());
        assert_eq!(session.tab_state_label().as_deref(), Some("Unavailable"));
        assert_eq!(
            session.status_message().as_deref(),
            Some("Terminal can no longer send input to this session. Existing output is readable.")
        );
        assert_eq!(session.write(b"retry"), Err(SessionWriteError::Write));
    }

    /// UIA-02: `tab_title`'s priority end to end — a fresh shell session
    /// defaults to "user — -shell", not the starting directory's name, but
    /// a real `cd` (a confirmed OSC 7 report) still outranks that default,
    /// exactly like the `terminal/title-follows-directory` behaviour
    /// scenario expects.
    #[test]
    fn tab_title_prefers_a_confirmed_directory_over_the_default_identity() {
        let size = TermSize { cols: 20, lines: 5 };
        let directory = SessionDirectory::from_local(std::path::Path::new("/home/jake/repo"));
        let session = Session {
            id: 1,
            term: Arc::new(Mutex::new(Term::new(
                terminal_config(10),
                &size,
                EventProxy::default(),
            ))),
            ui: SessionUiState::default(),
            accepted_size: size,
            transport: SessionTransportState::default(),
            writer: Arc::new(Mutex::new(
                Box::new(std::io::sink()) as Box<dyn Write + Send>
            )),
            write_failed: Arc::new(AtomicBool::new(false)),
            title: SessionTitle::default(),
            manual_title: SessionTitle::default(),
            default_identity: Some("jake — -zsh".to_string()),
            directory: directory.clone(),
            shell_state: SessionShellState::default(),
            scrollback_limit: Arc::new(AtomicUsize::new(10)),
            job_state: SessionJobState::default(),
            master: None,
            shell_pid: None,
            killer: None,
            lifecycle: Arc::new(Mutex::new(SessionLifecycle::Running)),
            origin: InitialProgram::default(),
        };

        // A fresh window: no job, no OSC title, and the directory is only
        // the unconfirmed starting guess — the default identity wins.
        assert_eq!(session.tab_title().as_deref(), Some("jake — -zsh"));

        // A real `cd` confirms the directory over OSC 7 — it now wins.
        assert!(directory.set_uri("file:///tmp"));
        assert_eq!(session.tab_title().as_deref(), Some("tmp"));
    }

    #[test]
    fn worker_resources_have_explicit_bounds() {
        assert_eq!(SESSION_WORKERS_PER_TAB, 2);
        assert_eq!(SESSION_WORKER_STACK_BYTES, 512 * 1024);
        assert_eq!(
            MAX_TABS * SESSION_WORKERS_PER_TAB * SESSION_WORKER_STACK_BYTES,
            16 * 1024 * 1024
        );
        assert_eq!(terminal_config(SCROLLBACK_LINES).scrolling_history, 10_000);
    }
}
