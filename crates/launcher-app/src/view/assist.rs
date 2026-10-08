//! "Lulo can do this" rows (ADR 0024 §1 feature 1, §7).
//!
//! Lulo Intelligence is asked only when all of this holds:
//!
//! - the user turned it on in System Settings ▸ Lulo Intelligence (read
//!   once, off the UI thread, when Spotlight opens);
//! - every search provider has answered and none matched confidently
//!   (no answer card, no result whose name starts with the query or one of
//!   its words);
//! - the query reads like a request: two or more words with letters;
//! - typing paused for [`DEBOUNCE`].
//!
//! The request runs on the blocking pool and a newer keystroke drops it, so
//! typing and drawing never wait for the model; the service also stops a
//! request of ours that a newer one replaced, at its next forward pass. An
//! "open <app>" request whose app name is an installed app's, a light typo
//! allowed ("opn notse"), is answered at once without the model. The answer becomes one
//! ordinary result row. Nothing changes until the row is picked, and a
//! Settings change asks for a second Return (or click) on the row itself.

use std::time::Duration;

use gpui::{Context, Task};
use rmac_intelligence::client::ClientError;
use rmac_intelligence::{prompt, Intent, Tier};
use rmac_launcher::{Action, Category, ResultId, SearchResult, INTELLIGENCE_PROVIDER};

use super::LauncherView;

/// Typing pause before the model is asked (ADR 0024 §7). 150 ms, down from
/// 250 ms in phase 1.1: a request the user types past is replaced in the
/// service and stops at its next forward pass, so a pause mid-word costs
/// the next request at most one prefill (p90 about 0.2 s on the reference
/// laptop), no more than the longer pause cost every request before.
pub(crate) const DEBOUNCE: Duration = Duration::from_millis(150);
pub(crate) const SUBTITLE: &str = "Lulo can do this";
pub(crate) const CONFIRM_SUBTITLE: &str = "Press Return again to confirm";

#[derive(Default)]
pub(crate) struct Assist {
    /// `None` until the setting is read; then whether to ask at all. A
    /// service that answers "off" or "not downloaded" turns it off for the
    /// rest of this Spotlight session.
    enabled: Option<bool>,
    /// The generation already considered, so each query is asked once.
    considered: Option<u64>,
    /// The request in flight; dropping it abandons the reply.
    pending: Option<Task<()>>,
    /// Whether the model was asked to load during this session.
    prepared: bool,
    /// The row on show, its generation and whether Return was pressed once.
    shown: Option<Shown>,
}

#[derive(Clone)]
struct Shown {
    generation: u64,
    result: SearchResult,
    intent: Intent,
    armed: bool,
}

/// What the request task brings back.
struct Answer {
    intent: Intent,
    app: Option<SearchResult>,
    /// What the service measured; `None` when no model was asked.
    timing: Option<rmac_intelligence::client::Timing>,
}

impl LauncherView {
    /// Read the on/off setting off the UI thread when Spotlight opens. If
    /// the model is on disk but its saved prompt state is not (an update
    /// changed the prompt, and Settings has not warmed it yet), ask the
    /// service to get ready at once, so the evaluation overlaps typing.
    pub(crate) fn load_assist_setting(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let (enabled, cold) = blocking::unblock(|| {
                let config = rmac_intelligence::config::Config::load();
                if !config.enabled {
                    return (false, false);
                }
                let facts = rmac_intelligence::gate::current_facts();
                let Some(tier) = config.tier(&facts) else {
                    return (true, false);
                };
                let model = tier.model();
                let present = rmac_intelligence::paths::models_dir()
                    .is_some_and(|models| rmac_intelligence::verify::is_present(&models, model));
                (
                    true,
                    present && !rmac_intelligence::prefix_state::is_ready(model),
                )
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                this.assist.enabled = Some(enabled);
                if cold {
                    this.prepare_assist();
                }
                this.consider_assist(cx);
            });
        })
        .detach();
    }

    /// Load the model now (once per Spotlight session), so it is ready by
    /// the time typing pauses.
    fn prepare_assist(&mut self) {
        if self.assist.prepared {
            return;
        }
        self.assist.prepared = true;
        blocking::unblock(|| {
            let _ = rmac_intelligence::client::prepare();
        })
        .detach();
    }

    /// Called whenever results change: ask the model if this query needs it.
    pub(crate) fn consider_assist(&mut self, cx: &mut Context<Self>) {
        if self.assist.enabled != Some(true) || self.browse_mode.is_some() || self.panel.is_some() {
            return;
        }
        let snapshot = self.coordinator.snapshot();
        if !snapshot.open || snapshot.pending_providers > 0 {
            return;
        }
        let generation = self.coordinator.generation();
        if self.assist.considered == Some(generation) {
            return;
        }
        self.assist.considered = Some(generation);
        self.assist.pending = None;
        let query = snapshot.query;
        if !prompt::worth_asking(&query) || self.coordinator.has_confident_match() {
            return;
        }
        self.prepare_assist();
        let applications = self.applications.clone();
        rmac_ui::trace_mark("assist_considered");
        self.assist.pending = Some(cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            cx.background_executor().timer(DEBOUNCE).await;
            let still_current = this
                .update(cx, |this, _| this.coordinator.generation() == generation)
                .unwrap_or(false);
            if !still_current {
                return;
            }
            rmac_ui::trace_mark("assist_asked");
            let answer = blocking::unblock(move || {
                // "opn notse": an open verb and an installed app's name,
                // each with a light typo, needs no model at all.
                if let Some(app) = crate::intelligence::open_request_app(&applications, &query) {
                    return Ok(Answer {
                        intent: Intent::OpenApp {
                            app: app.title.clone(),
                        },
                        app: Some(app),
                        timing: None,
                    });
                }
                let reply = rmac_intelligence::client::intent(&query)?;
                let app = match &reply.intent {
                    Intent::OpenApp { app } => crate::intelligence::resolve_app(&applications, app)
                        .or_else(|| crate::intelligence::resolve_app_typo(&applications, app)),
                    _ => None,
                };
                Ok::<_, ClientError>(Answer {
                    intent: reply.intent,
                    app,
                    timing: Some(reply.timing),
                })
            })
            .await;
            let _ = this.update(cx, |this, cx| this.show_assist(generation, answer, cx));
        }));
    }

    fn show_assist(
        &mut self,
        generation: u64,
        answer: Result<Answer, ClientError>,
        cx: &mut Context<Self>,
    ) {
        rmac_ui::trace_mark("assist_reply");
        let answer = match answer {
            Ok(answer) => answer,
            // A newer query already asked again.
            Err(ClientError::Cancelled) => return,
            Err(error) => {
                // The reason only, never what was typed.
                if error != ClientError::Off {
                    eprintln!("rmac-launcher: Lulo Intelligence did not answer: {error}");
                }
                if matches!(
                    error,
                    ClientError::Off
                        | ClientError::NotDownloaded
                        | ClientError::NotSupported
                        | ClientError::Unavailable
                ) {
                    self.assist.enabled = Some(false);
                }
                return;
            }
        };
        if let Some(timing) = &answer.timing {
            // For scripts/behavior/run_spotlight_intents.py: what the
            // service spent, on the same clock as the frames.
            rmac_ui::trace_mark(&format!(
                "assist_timing:total_ms={:.0}:queued_ms={:.0}:rewind_ms={:.0}:prefill_ms={:.0}:decode_ms={:.0}:request_tokens={}:passes={}",
                timing.total_ms,
                timing.queued_ms,
                timing.rewind_ms,
                timing.prefill_ms,
                timing.decode_ms,
                timing.request_tokens,
                timing.passes
            ));
        }
        let Some(result) = assist_row(&answer.intent, answer.app) else {
            return;
        };
        if self
            .coordinator
            .apply_assist(generation, Some(result.clone()))
        {
            self.assist.shown = Some(Shown {
                generation,
                result,
                intent: answer.intent,
                armed: false,
            });
            rmac_ui::trace_mark("assist_row_applied");
            cx.notify();
        }
    }

    /// A pick on `id`: true when it only armed a Settings change (the row
    /// now asks for a second Return), false when the pick should run.
    pub(crate) fn arm_assist(&mut self, id: &ResultId, cx: &mut Context<Self>) -> bool {
        let Some(shown) = self.assist.shown.as_mut() else {
            return false;
        };
        if &shown.result.id != id
            || shown.armed
            || shown.intent.tier() != Tier::Confirm
            || shown.generation != self.coordinator.generation()
        {
            return false;
        }
        shown.armed = true;
        let mut armed = shown.result.clone();
        armed.subtitle = Some(CONFIRM_SUBTITLE.into());
        let generation = shown.generation;
        if self.coordinator.apply_assist(generation, Some(armed)) {
            self.coordinator.select(id);
            cx.notify();
        }
        true
    }

    /// The selected row's id, if it is the assist row.
    pub(crate) fn selected_assist(&self) -> Option<ResultId> {
        self.coordinator
            .selected()
            .filter(|id| id.provider.0 == INTELLIGENCE_PROVIDER)
    }
}

/// The row for an intent: an app launch and a file search use the
/// launcher's own actions; everything else performs the intent. `None` for
/// "none", and for an app that is not installed.
pub(crate) fn assist_row(intent: &Intent, app: Option<SearchResult>) -> Option<SearchResult> {
    let (title, primary, icon) = match intent {
        Intent::None => return None,
        Intent::OpenApp { .. } => {
            let app = app?;
            (
                intent.title(Some(&app.title))?,
                app.primary.clone(),
                app.icon.clone(),
            )
        }
        Intent::SearchFiles { query } => (
            intent.title(None)?,
            Action::SearchFiles {
                query: query.clone(),
            },
            None,
        ),
        _ => (
            intent.title(None)?,
            Action::PerformIntent {
                intent: intent.clone(),
            },
            None,
        ),
    };
    Some(SearchResult {
        id: ResultId {
            provider: rmac_shell_settings::ProviderId(INTELLIGENCE_PROVIDER.into()),
            local: intent.to_json(),
        },
        category: Category::Intelligence,
        application_group: None,
        title,
        subtitle: Some(SUBTITLE.into()),
        detail: None,
        icon,
        primary,
        alternate: None,
        recency_rank: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_name_the_change_and_never_carry_none() {
        let dark = Intent::parse(r#"{"intent":"appearance","mode":"dark"}"#).unwrap();
        let row = assist_row(&dark, None).unwrap();
        assert_eq!(row.title, "Turn On Dark Mode");
        assert_eq!(row.subtitle.as_deref(), Some(SUBTITLE));
        assert_eq!(row.category, Category::Intelligence);
        assert_eq!(row.primary, Action::PerformIntent { intent: dark });
        assert!(assist_row(&Intent::None, None).is_none());
        // An app that is not installed gets no row.
        assert!(assist_row(
            &Intent::OpenApp {
                app: "Nothing".into()
            },
            None
        )
        .is_none());
        let search = Intent::SearchFiles {
            query: "invoice".into(),
        };
        assert_eq!(
            assist_row(&search, None).unwrap().primary,
            Action::SearchFiles {
                query: "invoice".into()
            }
        );
    }
}
