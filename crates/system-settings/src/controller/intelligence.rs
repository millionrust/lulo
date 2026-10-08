//! System Settings ▸ Lulo Intelligence (ADR 0024 §4, §5; parity SET-115):
//! the on/off switch, the hardware gate's verdict, the checksum-verified
//! model download and the one-time speed check that decides whether the
//! larger model is offered.
//!
//! Everything that reads the disk, starts the downloader or talks to the
//! service runs on the blocking pool; nothing here polls. Download progress
//! arrives line by line from the `rmac-intelligence-fetch` helper.
//!
//! The first request after a download would otherwise pay the one-time
//! evaluation of the prompt prefix (about 15 s on the reference laptop).
//! Settings pays it instead, in the background, whenever Lulo Intelligence
//! is on, the model is on disk and its saved prefix state is missing (after
//! the download, after turning it on, or after an update changed the
//! prompt or the model): the pane says "Getting ready…" until the service
//! has written the state, and Spotlight's first request is warm.

mod render;

use std::io::BufRead as _;
use std::sync::{Arc, Mutex};

use rmac_intelligence::config::{CalibrationRecord, Config};
use rmac_intelligence::gate::Facts;
use rmac_intelligence::manifest::Tier;

use super::*;

#[derive(Default)]
pub(in crate::controller) struct IntelligencePane {
    pub(in crate::controller) loaded: bool,
    loading: bool,
    pub(in crate::controller) config: Config,
    pub(in crate::controller) facts: Facts,
    /// Whether each tier's model is on disk and verified ([`tier_index`]).
    pub(in crate::controller) present: [bool; 2],
    /// Bytes of a partial download, per tier.
    pub(in crate::controller) partial: [u64; 2],
    pub(in crate::controller) download: Option<Download>,
    pub(in crate::controller) calibrating: bool,
    /// Whether each tier's prompt-prefix state is saved, so its first
    /// request is warm ([`rmac_intelligence::prefix_state`]).
    pub(in crate::controller) ready: [bool; 2],
    /// The service is evaluating and saving the prefix state now.
    pub(in crate::controller) preparing: bool,
    pub(in crate::controller) busy: bool,
    pub(in crate::controller) error: Option<SharedString>,
}

pub(in crate::controller) struct Download {
    pub(in crate::controller) tier: Tier,
    pub(in crate::controller) done: u64,
    pub(in crate::controller) total: u64,
    child: Arc<Mutex<Option<std::process::Child>>>,
}

pub(in crate::controller) fn tier_index(tier: Tier) -> usize {
    match tier {
        Tier::Tiny => 0,
        Tier::Standard => 1,
    }
}

struct Loaded {
    config: Config,
    facts: Facts,
    present: [bool; 2],
    partial: [u64; 2],
    ready: [bool; 2],
}

fn load() -> Loaded {
    let config = Config::load();
    let facts = rmac_intelligence::gate::current_facts();
    let models = rmac_intelligence::paths::models_dir();
    let present = Tier::ALL.map(|tier| {
        models
            .as_deref()
            .is_some_and(|models| rmac_intelligence::verify::is_present(models, tier.model()))
    });
    let partial = Tier::ALL.map(|tier| rmac_intelligence::fetch::partial_bytes(tier.model()));
    Loaded {
        config,
        facts,
        present,
        partial,
        ready: prefix_states(),
    }
}

fn prefix_states() -> [bool; 2] {
    Tier::ALL.map(|tier| rmac_intelligence::prefix_state::is_ready(tier.model()))
}

/// What a line from `rmac-intelligence-fetch` says.
#[derive(Debug, PartialEq)]
pub(in crate::controller) enum FetchLine {
    Progress(u64, u64),
    Done,
    Failed(String),
}

pub(in crate::controller) fn parse_fetch_line(line: &str) -> Option<FetchLine> {
    let mut words = line.splitn(3, ' ');
    match words.next()? {
        "progress" => {
            let done = words.next()?.parse().ok()?;
            let total = words.next()?.trim().parse().ok()?;
            Some(FetchLine::Progress(done, total))
        }
        "done" => Some(FetchLine::Done),
        "error" => Some(FetchLine::Failed(
            line.trim_start_matches("error").trim().to_owned(),
        )),
        _ => None,
    }
}

impl Settings {
    /// The tier this PC uses: the user's choice where the gate allows it.
    pub(in crate::controller) fn intelligence_tier(&self) -> Tier {
        let pane = &self.intelligence;
        pane.config.tier(&pane.facts).unwrap_or(Tier::Tiny)
    }

    pub(in crate::controller) fn refresh_intelligence(&mut self, cx: &mut Context<Self>) {
        if self.intelligence.loading {
            return;
        }
        self.intelligence.loading = true;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let loaded = blocking::unblock(load).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                let pane = &mut this.intelligence;
                pane.loading = false;
                pane.loaded = true;
                pane.config = loaded.config;
                pane.facts = loaded.facts;
                pane.present = loaded.present;
                pane.partial = loaded.partial;
                pane.ready = loaded.ready;
                this.warm_intelligence(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn save_intelligence_config(&mut self, config: Config, cx: &mut Context<Self>) {
        self.intelligence.busy = true;
        self.intelligence.error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let saved = config.clone();
            let result = blocking::unblock(move || saved.save()).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.intelligence.busy = false;
                match result {
                    Ok(()) => {
                        this.intelligence.config = config;
                        this.warm_intelligence(cx);
                    }
                    Err(_) => {
                        this.intelligence.error =
                            Some("Could not save the Lulo Intelligence setting.".into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::controller) fn set_intelligence_enabled(
        &mut self,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        let mut config = self.intelligence.config.clone();
        config.enabled = enabled;
        self.save_intelligence_config(config, cx);
    }

    pub(in crate::controller) fn choose_intelligence_tier(
        &mut self,
        tier: Tier,
        cx: &mut Context<Self>,
    ) {
        let mut config = self.intelligence.config.clone();
        config.tier = Some(tier.as_str().to_owned());
        self.save_intelligence_config(config, cx);
    }

    /// Start (or resume) the checksum-verified download of the chosen model
    /// in the `rmac-intelligence-fetch` helper process.
    pub(in crate::controller) fn download_intelligence_model(&mut self, cx: &mut Context<Self>) {
        if self.intelligence.download.is_some() {
            return;
        }
        let tier = self.intelligence_tier();
        let model = tier.model();
        let child = Arc::new(Mutex::new(None));
        self.intelligence.download = Some(Download {
            tier,
            done: self.intelligence.partial[tier_index(tier)],
            total: model.size,
            child: child.clone(),
        });
        self.intelligence.error = None;
        cx.notify();
        let (lines_tx, lines) = async_channel::bounded::<FetchLine>(16);
        // Spawning a child from GPUI's background executor is not safe
        // (LINUX-HW-07); the blocking pool both starts and reads it.
        blocking::unblock(move || {
            // Beside Settings in a development install; the session
            // package keeps it in /usr/libexec/rmac while Settings itself
            // may run from /usr/bin.
            let executable = std::env::current_exe()
                .ok()
                .map(|path| path.with_file_name("rmac-intelligence-fetch"))
                .into_iter()
                .chain([std::path::PathBuf::from(
                    "/usr/libexec/rmac/rmac-intelligence-fetch",
                )])
                .find(|path| path.is_file());
            let Some(executable) = executable else {
                let _ = lines_tx
                    .send_blocking(FetchLine::Failed("the downloader is not installed".into()));
                return;
            };
            let spawned = std::process::Command::new(executable)
                .arg(tier.as_str())
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn();
            let mut spawned = match spawned {
                Ok(spawned) => spawned,
                Err(_) => {
                    let _ = lines_tx
                        .send_blocking(FetchLine::Failed("the downloader is not installed".into()));
                    return;
                }
            };
            let stdout = spawned.stdout.take();
            if let Ok(mut slot) = child.lock() {
                *slot = Some(spawned);
            }
            let mut finished = false;
            if let Some(stdout) = stdout {
                for line in std::io::BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    if let Some(parsed) = parse_fetch_line(&line) {
                        finished |= !matches!(parsed, FetchLine::Progress(..));
                        if lines_tx.send_blocking(parsed).is_err() {
                            break;
                        }
                    }
                }
            }
            if let Ok(mut slot) = child.lock() {
                if let Some(mut process) = slot.take() {
                    let _ = process.wait();
                }
            }
            if !finished {
                let _ = lines_tx.send_blocking(FetchLine::Failed("the download stopped".into()));
            }
        })
        .detach();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(line) = lines.recv().await {
                let stop = !matches!(line, FetchLine::Progress(..));
                let alive = this.update(cx, |this: &mut Settings, cx| {
                    this.apply_fetch_line(line, cx);
                    cx.notify();
                });
                if alive.is_err() || stop {
                    break;
                }
            }
        })
        .detach();
    }

    fn apply_fetch_line(&mut self, line: FetchLine, cx: &mut Context<Self>) {
        match line {
            FetchLine::Progress(done, total) => {
                if let Some(download) = self.intelligence.download.as_mut() {
                    download.done = done;
                    download.total = total.max(1);
                }
            }
            FetchLine::Done => {
                self.intelligence.download = None;
                // The refresh sees the model and warms it up.
                self.refresh_intelligence(cx);
            }
            FetchLine::Failed(message) => {
                self.intelligence.download = None;
                self.intelligence.error = Some(format!("The download stopped: {message}.").into());
                self.refresh_intelligence(cx);
            }
        }
    }

    /// Stop the download; the partial file stays, so it can resume.
    pub(in crate::controller) fn cancel_intelligence_download(&mut self, cx: &mut Context<Self>) {
        if let Some(download) = &self.intelligence.download {
            let child = download.child.clone();
            blocking::unblock(move || {
                if let Ok(mut slot) = child.lock() {
                    if let Some(process) = slot.as_mut() {
                        let _ = process.kill();
                    }
                }
            })
            .detach();
        }
        cx.notify();
    }

    pub(in crate::controller) fn remove_intelligence_model(&mut self, cx: &mut Context<Self>) {
        let tier = self.intelligence_tier();
        self.intelligence.busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result =
                blocking::unblock(move || rmac_intelligence::fetch::remove(tier.model())).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.intelligence.busy = false;
                if result.is_err() {
                    this.intelligence.error = Some("Could not remove the model.".into());
                }
                this.refresh_intelligence(cx);
            });
        })
        .detach();
    }

    /// Make the next Spotlight request warm: when Lulo Intelligence is on
    /// and the chosen model is on disk, measure this PC's speed if that was
    /// never done (which also loads the model and saves its prompt state),
    /// or else ask the service to load and save the state (`Prepare`) if it
    /// is missing. Nothing happens when the state is already saved.
    pub(in crate::controller) fn warm_intelligence(&mut self, cx: &mut Context<Self>) {
        let pane = &self.intelligence;
        let index = tier_index(self.intelligence_tier());
        let offered = !matches!(
            pane.config.decision(&pane.facts),
            rmac_intelligence::gate::Decision::NotOffered(_)
        );
        if !pane.loaded
            || !pane.config.enabled
            || !offered
            || !pane.present[index]
            || pane.download.is_some()
            || pane.preparing
            || pane.calibrating
        {
            return;
        }
        if pane.config.calibration_for(&pane.facts).is_none() {
            self.calibrate_intelligence(cx);
        } else if !pane.ready[index] {
            self.prepare_intelligence(cx);
        }
    }

    fn prepare_intelligence(&mut self, cx: &mut Context<Self>) {
        self.intelligence.preparing = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(|| {
                let prepared = rmac_intelligence::client::prepare();
                (prepared, prefix_states())
            })
            .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                let (prepared, ready) = result;
                this.intelligence.preparing = false;
                this.intelligence.ready = ready;
                if let Err(error) = prepared {
                    this.intelligence.error =
                        Some(format!("Lulo Intelligence could not get ready: {error}.").into());
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Measure this PC's decode speed once (the hardware gate's floor), and
    /// keep it with the hardware it was measured on.
    pub(in crate::controller) fn calibrate_intelligence(&mut self, cx: &mut Context<Self>) {
        if self.intelligence.calibrating {
            return;
        }
        self.intelligence.calibrating = true;
        cx.notify();
        let facts = self.intelligence.facts.clone();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(move || {
                let reply =
                    rmac_intelligence::client::calibrate().map_err(|error| error.to_string())?;
                let mut config = Config::load();
                let mut record = config.calibration.clone().unwrap_or_default();
                if record.cpu_model != facts.cpu_model
                    || record.memory_total_mib != facts.memory_total_mib
                {
                    record = CalibrationRecord::default();
                }
                record.cpu_model = facts.cpu_model.clone();
                record.memory_total_mib = facts.memory_total_mib;
                if reply.tier == Tier::Standard.as_str() {
                    record.standard_decode_tok_s = Some(reply.decode_tok_s);
                } else {
                    record.tiny_decode_tok_s = reply.decode_tok_s;
                }
                config.calibration = Some(record);
                config.save().map_err(|error| error.to_string())?;
                Ok::<_, String>(config)
            })
            .await;
            // Calibrating loaded the model, so its prompt state is saved now.
            let ready = blocking::unblock(prefix_states).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.intelligence.calibrating = false;
                this.intelligence.ready = ready;
                match result {
                    Ok(config) => this.intelligence.config = config,
                    Err(message) => {
                        this.intelligence.error =
                            Some(format!("Could not check this PC’s speed: {message}.").into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_lines_parse() {
        assert_eq!(
            parse_fetch_line("progress 10 532517120"),
            Some(FetchLine::Progress(10, 532_517_120))
        );
        assert_eq!(parse_fetch_line("done"), Some(FetchLine::Done));
        assert_eq!(
            parse_fetch_line("error the download failed; check the connection"),
            Some(FetchLine::Failed(
                "the download failed; check the connection".into()
            ))
        );
        assert_eq!(parse_fetch_line("progress x"), None);
        assert_eq!(parse_fetch_line(""), None);
    }
}
