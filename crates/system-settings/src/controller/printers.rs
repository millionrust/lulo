//! Printers & Scanners: CUPS reads and cups-pk-helper changes, all on
//! blocking workers. Reads refresh when the pane opens and after every
//! change; nothing polls.

use super::*;
mod render;

use rmac_printers_linux::{Cups, Device, Job, PaperSize, Preferences, Printer, PrinterAdmin};

/// How long CUPS's backends browse for printers in Add Printer.
const DISCOVERY_SECONDS: i32 = 8;

#[derive(Default)]
pub(super) struct PrintersState {
    pub(super) list: Vec<Printer>,
    /// The user's default printer (lpoptions), else the system default.
    pub(super) default: Option<String>,
    pub(super) paper: Option<PaperSize>,
    pub(super) loading: bool,
    pub(super) loaded: bool,
    pub(super) busy: bool,
    pub(super) error: Option<SharedString>,
    pub(super) info: Option<String>,
    pub(super) add: Option<AddSheet>,
    pub(super) queue: Option<QueueSheet>,
    pub(super) remove: Option<String>,
}

impl PrintersState {
    pub(super) fn printer(&self, name: &str) -> Option<&Printer> {
        self.list.iter().find(|printer| printer.name == name)
    }
}

pub(super) struct AddSheet {
    pub(super) searching: bool,
    pub(super) devices: Vec<Device>,
    pub(super) selected: Option<usize>,
    pub(super) name: Entity<InputState>,
    pub(super) location: Entity<InputState>,
    pub(super) error: Option<SharedString>,
    generation: u64,
}

pub(super) struct QueueSheet {
    pub(super) printer: String,
    pub(super) jobs: Vec<Job>,
    pub(super) loading: bool,
    pub(super) error: Option<SharedString>,
}

struct Snapshot {
    printers: Vec<Printer>,
    default: Option<String>,
    paper: PaperSize,
}

fn read_printers() -> Result<Snapshot, rmac_printers_linux::Error> {
    let cups = Cups::local();
    let printers = cups.printers()?;
    let prefs = Preferences::for_user();
    let default = prefs
        .as_ref()
        .and_then(Preferences::default_printer)
        .filter(|name| printers.iter().any(|printer| &printer.name == name))
        .or_else(|| cups.server_default().ok().flatten());
    let paper = prefs
        .as_ref()
        .map(Preferences::paper_size)
        .unwrap_or(PaperSize::A4);
    Ok(Snapshot {
        printers,
        default,
        paper,
    })
}

static NEXT_DISCOVERY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl Settings {
    pub(super) fn refresh_printers(&mut self, cx: &mut Context<Self>) {
        self.printers.loading = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(read_printers).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                let state = &mut this.printers;
                state.loading = false;
                state.loaded = true;
                match result {
                    Ok(snapshot) => {
                        state.list = snapshot.printers;
                        state.default = snapshot.default;
                        state.paper = Some(snapshot.paper);
                        state.error = None;
                        if state.info.as_ref().is_some_and(|name| {
                            state.list.iter().all(|printer| &printer.name != name)
                        }) {
                            state.info = None;
                        }
                    }
                    Err(error) => {
                        state.list.clear();
                        state.error = Some(error.message().into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Run one change on a worker; report its error; re-read.
    fn change_printers(
        &mut self,
        work: impl FnOnce() -> Result<(), rmac_printers_linux::Error> + Send + 'static,
        done: impl FnOnce(&mut Settings, Result<(), rmac_printers_linux::Error>) + 'static,
        cx: &mut Context<Self>,
    ) {
        if self.printers.busy {
            return;
        }
        self.printers.busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(work).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.printers.busy = false;
                done(this, result);
                this.refresh_printers(cx);
                this.refresh_print_queue(cx);
            });
        })
        .detach();
    }

    pub(super) fn set_default_printer(&mut self, name: String, cx: &mut Context<Self>) {
        self.change_printers(
            move || {
                Preferences::for_user()
                    .ok_or(rmac_printers_linux::Error::Failed)?
                    .set_default_printer(&name)
                    .map_err(|_| rmac_printers_linux::Error::Failed)
            },
            |this, result| {
                if let Err(error) = result {
                    this.printers.error = Some(error.message().into());
                }
            },
            cx,
        );
    }

    pub(super) fn set_paper_size(&mut self, size: PaperSize, cx: &mut Context<Self>) {
        let printers: Vec<String> = self
            .printers
            .list
            .iter()
            .map(|printer| printer.name.clone())
            .collect();
        self.change_printers(
            move || {
                Preferences::for_user()
                    .ok_or(rmac_printers_linux::Error::Failed)?
                    .set_paper_size(size, &printers)
                    .map_err(|_| rmac_printers_linux::Error::Failed)
            },
            |this, result| {
                if let Err(error) = result {
                    this.printers.error = Some(error.message().into());
                }
            },
            cx,
        );
    }

    pub(super) fn close_printer_sheets(&mut self, cx: &mut Context<Self>) {
        self.printers.info = None;
        self.printers.add = None;
        self.printers.queue = None;
        if !self.printers.busy {
            self.printers.remove = None;
        }
        cx.notify();
    }

    pub(super) fn open_add_printer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let generation = NEXT_DISCOVERY.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.printers.add = Some(AddSheet {
            searching: true,
            devices: Vec::new(),
            selected: None,
            name: cx.new(|cx| InputState::new(window, cx).placeholder("Name")),
            location: cx.new(|cx| InputState::new(window, cx).placeholder("Optional")),
            error: None,
            generation,
        });
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result =
                blocking::unblock(|| PrinterAdmin::system()?.discover(DISCOVERY_SECONDS)).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                let Some(sheet) = this
                    .printers
                    .add
                    .as_mut()
                    .filter(|sheet| sheet.generation == generation)
                else {
                    return;
                };
                sheet.searching = false;
                match result {
                    Ok(devices) => sheet.devices = devices,
                    Err(error) => sheet.error = Some(error.message().into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn select_new_printer(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(sheet) = self.printers.add.as_mut() else {
            return;
        };
        let Some(device) = sheet.devices.get(index) else {
            return;
        };
        let suggestion = rmac_printers_linux::model::suggest_printer_name(device.display_name());
        let location = device.location.clone();
        sheet.selected = Some(index);
        sheet.error = None;
        sheet
            .name
            .update(cx, |state, cx| state.set_value(suggestion, window, cx));
        sheet
            .location
            .update(cx, |state, cx| state.set_value(location, window, cx));
        cx.notify();
    }

    pub(super) fn submit_add_printer(&mut self, cx: &mut Context<Self>) {
        let Some(sheet) = self.printers.add.as_mut() else {
            return;
        };
        let Some(device) = sheet
            .selected
            .and_then(|index| sheet.devices.get(index))
            .cloned()
        else {
            sheet.error = Some("Choose a printer to add.".into());
            cx.notify();
            return;
        };
        let name = sheet.name.read(cx).value().trim().to_owned();
        let location = sheet.location.read(cx).value().trim().to_owned();
        if !rmac_printers_linux::model::validate_printer_name(&name) {
            sheet.error = Some(
                "The name can’t contain spaces, slashes, quotes or the characters # ? and ,."
                    .into(),
            );
            cx.notify();
            return;
        }
        if self.printers.printer(&name).is_some() {
            if let Some(sheet) = self.printers.add.as_mut() {
                sheet.error = Some("A printer with this name already exists.".into());
            }
            cx.notify();
            return;
        }
        self.change_printers(
            move || PrinterAdmin::system()?.add_printer(&name, &device, &location),
            |this, result| match result {
                Ok(()) => this.printers.add = None,
                Err(error) => {
                    if let Some(sheet) = this.printers.add.as_mut() {
                        sheet.error = Some(error.message().into());
                    }
                }
            },
            cx,
        );
    }

    pub(super) fn confirm_remove_printer(&mut self, cx: &mut Context<Self>) {
        let Some(name) = self.printers.remove.clone() else {
            return;
        };
        self.change_printers(
            move || PrinterAdmin::system()?.delete_printer(&name),
            |this, result| {
                this.printers.remove = None;
                this.printers.info = None;
                if let Err(error) = result {
                    this.printers.error = Some(error.message().into());
                }
            },
            cx,
        );
    }

    pub(super) fn open_print_queue(&mut self, name: String, cx: &mut Context<Self>) {
        self.printers.info = None;
        self.printers.queue = Some(QueueSheet {
            printer: name,
            jobs: Vec::new(),
            loading: true,
            error: None,
        });
        self.refresh_print_queue(cx);
    }

    fn refresh_print_queue(&mut self, cx: &mut Context<Self>) {
        let Some(name) = self
            .printers
            .queue
            .as_ref()
            .map(|queue| queue.printer.clone())
        else {
            return;
        };
        cx.notify();
        let printer = name.clone();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(move || Cups::local().jobs(&printer)).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if let Some(queue) = this
                    .printers
                    .queue
                    .as_mut()
                    .filter(|queue| queue.printer == name)
                {
                    queue.loading = false;
                    match result {
                        Ok(jobs) => {
                            queue.jobs = jobs;
                            queue.error = None;
                        }
                        Err(error) => queue.error = Some(error.message().into()),
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(super) fn cancel_print_job(&mut self, job: i32, cx: &mut Context<Self>) {
        self.change_printers(
            move || PrinterAdmin::system()?.cancel_job(job),
            |this, result| {
                if let (Err(error), Some(queue)) = (result, this.printers.queue.as_mut()) {
                    queue.error = Some(error.message().into());
                }
            },
            cx,
        );
    }

    /// Pause (stop) or resume the printer the queue sheet shows.
    pub(super) fn set_printer_paused(
        &mut self,
        name: String,
        paused: bool,
        cx: &mut Context<Self>,
    ) {
        self.change_printers(
            move || PrinterAdmin::system()?.set_enabled(&name, !paused),
            |this, result| {
                if let (Err(error), Some(queue)) = (result, this.printers.queue.as_mut()) {
                    queue.error = Some(error.message().into());
                }
            },
            cx,
        );
    }
}
