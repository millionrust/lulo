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
//! typing and drawing never wait for the model. The answer becomes one
//! ordinary result row. Nothing changes until the row is picked, and a
//! Settings change asks for a second Return (or click) on the row itself.

use std::time::Duration;

use gpui::{Context, Task};
use rmac_intelligence::client::ClientError;
use rmac_intelligence::{prompt, Intent, Tier};
use rmac_launcher::{Action, Category, ResultId, SearchResult, INTELLIGENCE_PROVIDER};

use super::LauncherView;

/// Typing pause before the model is asked (ADR 0024 §7).
pub(crate) const DEBOUNCE: Duration = Duration::from_millis(250);
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
}

impl LauncherView {
    /// Read the on/off setting off the UI thread when Spotlight opens.
    pub(crate) fn load_assist_setting(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let enabled =
                blocking::unblock(|| rmac_intelligence::config::Config::load().enabled).await;
            let _ = this.update(cx, |this, cx| {
                this.assist.enabled = Some(enabled);
                this.consider_assist(cx);
            });
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
        if !self.assist.prepared {
            // Load the model now so it is ready by the time typing pauses.
            self.assist.prepared = true;
            blocking::unblock(|| {
                let _ = rmac_intelligence::client::prepare();
            })
            .detach();
        }
        let applications = self.applications.clone();
        self.assist.pending = Some(cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            cx.background_executor().timer(DEBOUNCE).await;
            let still_current = this
                .update(cx, |this, _| this.coordinator.generation() == generation)
                .unwrap_or(false);
            if !still_current {
                return;
            }
            let answer = blocking::unblock(move || {
                let reply = rmac_intelligence::client::intent(&query)?;
                let app = match &reply.intent {
                    Intent::OpenApp { app } => crate::intelligence::resolve_app(&applications, app),
                    _ => None,
                };
                Ok::<_, ClientError>(Answer {
                    intent: reply.intent,
                    app,
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
        let answer = match answer {
            Ok(answer) => answer,
            Err(
                ClientError::Off
                | ClientError::NotDownloaded
                | ClientError::NotSupported
                | ClientError::Unavailable,
            ) => {
                self.assist.enabled = Some(false);
                return;
            }
            Err(_) => return,
        };
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
