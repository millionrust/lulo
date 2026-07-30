use std::io::{Read, Write};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc::{sync_channel, SyncSender},
    Arc, Mutex,
};
use std::thread::JoinHandle;

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::Processor;
use portable_pty::{
    native_pty_system, Child, ChildKiller, CommandBuilder, ExitStatus, MasterPty, PtySize,
};

use crate::emulator::{advance_filtered_output, terminal_config, TermSize};
use crate::output_filter::OutputFilter;
use crate::paste::{has_unsafe_unbracketed_control, logical_line_count, prepare as prepare_paste};

use crate::ui_state::SessionUiState;

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
}

impl EventProxy {
    fn connected(writer: SharedWriter, write_failed: Arc<AtomicBool>) -> Self {
        Self {
            writer: Some(writer),
            write_failed: Some(write_failed),
        }
    }
}

impl EventListener for EventProxy {
    fn send_event(&self, event: Event) {
        let Event::PtyWrite(text) = event else {
            return;
        };
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
}

fn request_redraw(redraw: &RedrawSender) {
    let _ = redraw.try_send(());
}

fn shell_program(configured: Option<String>) -> String {
    configured
        .filter(|shell| !shell.trim().is_empty())
        .unwrap_or_else(|| "/bin/sh".to_string())
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SessionLifecycle {
    Running,
    Exited {
        exit_code: u32,
        signal: Option<String>,
    },
    WaitFailed,
    StartFailed,
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
            Self::StartFailed => Some("Terminal could not start the configured shell.".into()),
        }
    }

    fn tab_state_label(&self) -> Option<&'static str> {
        match self {
            Self::Running => None,
            Self::Exited { .. } => Some("Exited"),
            Self::WaitFailed | Self::StartFailed => Some("Unavailable"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SessionStartError {
    OpenPty,
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
            Self::StartShell => "Terminal could not start the configured shell.",
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
    RedrawSender,
);
type WaiterTask = (
    Box<dyn Child + Send + Sync>,
    Arc<Mutex<SessionLifecycle>>,
    RedrawSender,
);

fn run_reader_worker((mut reader, term, redraw): ReaderTask) {
    let mut parser: Processor = Processor::new();
    let mut output_filter = OutputFilter::default();
    let mut buffer = [0u8; 8192];
    let mut filtered = Vec::with_capacity(buffer.len());
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                output_filter.filter_into(&buffer[..read], &mut filtered);
                if filtered.is_empty() {
                    continue;
                }
                if let Ok(mut term) = term.lock() {
                    advance_filtered_output(&mut parser, &mut *term, &filtered);
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
    master: Option<Box<dyn MasterPty + Send>>,
    shell_pid: Option<u32>,
    killer: Option<Box<dyn ChildKiller + Send + Sync>>,
    lifecycle: Arc<Mutex<SessionLifecycle>>,
}

impl Session {
    pub(super) fn spawn(
        cols: usize,
        rows: usize,
        scrollback_lines: usize,
        redraw: RedrawSender,
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
        let workers = ReservedSessionWorkers::reserve()?;
        let shell = shell_program(std::env::var("SHELL").ok());
        let mut command = CommandBuilder::new(shell);
        command.env("TERM", "xterm-256color");
        if let Ok(directory) = std::env::current_dir() {
            command.cwd(directory);
        }
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|_| SessionStartError::StartShell)?;
        let shell_pid = child.process_id();
        let mut killer = child.clone_killer();
        drop(pair.slave);
        let term = Arc::new(Mutex::new(Term::new(
            terminal_config(scrollback_lines),
            &size,
            EventProxy::connected(Arc::clone(&writer), Arc::clone(&write_failed)),
        )));
        let lifecycle = Arc::new(Mutex::new(SessionLifecycle::Running));
        workers.activate(
            (reader, Arc::clone(&term), redraw.clone()),
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
            master: Some(pair.master),
            shell_pid,
            killer: Some(killer),
            lifecycle,
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
            master: None,
            shell_pid: None,
            killer: None,
            lifecycle: Arc::new(Mutex::new(SessionLifecycle::StartFailed)),
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

    pub(super) fn tab_state_label(&self) -> Option<&'static str> {
        if self.lifecycle().is_running() && self.write_failed.load(Ordering::Acquire) {
            Some("Unavailable")
        } else {
            self.lifecycle().tab_state_label()
        }
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
        Ok(())
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
        let size = TermSize { cols: 20, lines: 5 };
        let mut term = Term::new(
            terminal_config(10),
            &size,
            EventProxy::connected(writer, Arc::clone(&write_failed)),
        );
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
        assert_eq!(shell_program(None), "/bin/sh");
        assert_eq!(shell_program(Some("  ".to_string())), "/bin/sh");
        assert_eq!(shell_program(Some("/bin/fish".to_string())), "/bin/fish");
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
        assert!(!SessionLifecycle::StartFailed.may_be_running());
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
            master: None,
            shell_pid: None,
            killer: None,
            lifecycle: Arc::new(Mutex::new(SessionLifecycle::Running)),
        };

        assert!(session.accepts_input());
        assert_eq!(session.write(b"private"), Err(SessionWriteError::Write));
        assert!(!session.accepts_input());
        assert_eq!(session.tab_state_label(), Some("Unavailable"));
        assert_eq!(
            session.status_message().as_deref(),
            Some("Terminal can no longer send input to this session. Existing output is readable.")
        );
        assert_eq!(session.write(b"retry"), Err(SessionWriteError::Write));
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
