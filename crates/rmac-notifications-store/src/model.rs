//! Notification Center history, policy, and lock-preview model.

use super::*;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LockPreview {
    Show,
    HideContent,
    Hide,
}

/// Bounded, action-free data that a secure session-lock client may render.
///
/// `content=None` intentionally reveals only application identity and recency.
/// Action identifiers, targets, categories, sounds, and replacement identities
/// never cross this boundary.
#[derive(Clone, Eq, PartialEq)]
pub struct LockPreviewRecord {
    pub notification_id: NotificationId,
    pub app_id: AppId,
    pub content: Option<Content>,
    pub updated_at: Time,
}

impl fmt::Debug for LockPreviewRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LockPreviewRecord")
            .field("notification_id", &self.notification_id)
            .field("app_id", &"<redacted>")
            .field("content", &self.content.as_ref().map(|_| "<redacted>"))
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct AppPolicy {
    pub enabled: bool,
    pub banners: bool,
    pub sounds: bool,
    pub badges: bool,
    pub history: bool,
    pub urgent_through_focus: bool,
    pub lock_preview: LockPreview,
}

impl Default for AppPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            banners: true,
            sounds: true,
            badges: true,
            history: true,
            urgent_through_focus: true,
            lock_preview: LockPreview::Hide,
        }
    }
}

impl AppPolicy {
    pub fn delivery(self, focus_active: bool) -> DeliveryPolicy {
        DeliveryPolicy {
            enabled: self.enabled,
            banner: if self.banners {
                BannerPolicy::Allow
            } else {
                BannerPolicy::Suppress
            },
            sounds: self.sounds,
            history: if self.history {
                HistoryPolicy::Allow
            } else {
                HistoryPolicy::Block
            },
            allow_urgent_through_focus: self.urgent_through_focus,
            focus_active,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Recovery {
    #[default]
    None,
    LastGood,
    Empty,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadSnapshot {
    pub center: Center,
    pub recovery: Recovery,
}

#[derive(Clone, Default, Eq, PartialEq)]
pub struct Center {
    pub(super) history: Vec<Notification>,
    pub(super) policies: BTreeMap<AppId, AppPolicy>,
}

impl fmt::Debug for Center {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Center")
            .field(
                "history",
                &format_args!("<{} redacted records>", self.history.len()),
            )
            .field(
                "policies",
                &format_args!("<{} redacted apps>", self.policies.len()),
            )
            .finish()
    }
}

impl Center {
    pub fn history(&self) -> &[Notification] {
        &self.history
    }

    pub fn policy(&self, app_id: &AppId) -> AppPolicy {
        self.policies.get(app_id).copied().unwrap_or_default()
    }

    pub fn set_policy(&mut self, app_id: AppId, policy: AppPolicy) -> Result<bool, Error> {
        if !self.policies.contains_key(&app_id) && self.policies.len() >= MAX_POLICIES {
            return Err(Error::new(Operation::Validate, ErrorKind::Limit));
        }
        let changed = self.policies.get(&app_id).copied() != Some(policy);
        self.policies.insert(app_id.clone(), policy);
        if !policy.history || !policy.enabled {
            self.clear(Some(&app_id));
        }
        Ok(changed)
    }

    pub fn policies(&self) -> impl Iterator<Item = (&AppId, &AppPolicy)> {
        self.policies.iter()
    }

    pub fn applications(&self) -> Vec<(AppId, AppPolicy)> {
        let app_ids: BTreeSet<_> = self
            .policies
            .keys()
            .chain(self.history.iter().map(|record| record.source.app_id()))
            .cloned()
            .collect();
        app_ids
            .into_iter()
            .map(|app_id| {
                let policy = self.policy(&app_id);
                (app_id, policy)
            })
            .collect()
    }

    /// Project newest unread records through both application hints and the
    /// user's stricter per-app policy. The provider may request fewer records,
    /// but can never exceed the security boundary's fixed maximum.
    pub fn lock_previews(&self, requested: usize) -> Vec<LockPreviewRecord> {
        let limit = requested.min(MAX_LOCK_PREVIEWS);
        self.history
            .iter()
            .rev()
            .filter(|record| record.unread)
            .filter_map(|record| {
                let policy = self.policy(record.source.app_id());
                if !policy.enabled || !policy.history {
                    return None;
                }
                let preview =
                    restrict_lock_preview(policy.lock_preview, record.display.lock_screen);
                (preview != LockPreview::Hide).then(|| LockPreviewRecord {
                    notification_id: record.id,
                    app_id: record.source.app_id().clone(),
                    content: (preview == LockPreview::Show).then(|| record.content.clone()),
                    updated_at: record.updated_at,
                })
            })
            .take(limit)
            .collect()
    }

    pub fn upsert(&mut self, notification: Notification) {
        if !notification.delivery.history {
            return;
        }
        let app_id = notification.source.app_id().clone();
        let policy = self.policy(&app_id);
        if !policy.enabled || !policy.history {
            self.clear(Some(&app_id));
            return;
        }
        self.history
            .retain(|record| record.id != notification.id || record.source != notification.source);
        self.history.push(notification);
        self.enforce_bounds(&app_id);
    }

    pub fn clear(&mut self, app_id: Option<&AppId>) -> bool {
        let before = self.history.len();
        match app_id {
            Some(app_id) => self
                .history
                .retain(|record| record.source.app_id() != app_id),
            None => self.history.clear(),
        }
        self.history.len() != before
    }

    pub fn remove(&mut self, id: NotificationId) -> bool {
        let before = self.history.len();
        self.history.retain(|record| record.id != id);
        self.history.len() != before
    }

    pub fn mark_all_read(&mut self, app_id: Option<&AppId>) -> bool {
        let mut changed = false;
        for record in &mut self.history {
            if app_id.is_none_or(|app_id| record.source.app_id() == app_id) {
                changed |= record.unread;
                record.unread = false;
            }
        }
        changed
    }

    pub fn indicator(&self) -> Indicator {
        let unread = self
            .history
            .iter()
            .filter(|record| record.unread && self.policy(record.source.app_id()).badges);
        Indicator {
            unread_count: unread.clone().count().try_into().unwrap_or(u32::MAX),
            has_urgent: unread
                .into_iter()
                .any(|record| record.priority == Priority::Urgent),
        }
    }

    pub fn groups(&self) -> Vec<Group<'_>> {
        let mut app_ids = BTreeSet::new();
        for record in &self.history {
            app_ids.insert(record.source.app_id());
        }
        let mut groups: Vec<_> = app_ids
            .into_iter()
            .map(|app_id| {
                let mut records: Vec<_> = self
                    .history
                    .iter()
                    .filter(|record| record.source.app_id() == app_id)
                    .collect();
                records.sort_by_key(|record| std::cmp::Reverse(record.updated_at.0));
                Group { app_id, records }
            })
            .collect();
        groups.sort_by_key(|group| {
            std::cmp::Reverse(
                group
                    .records
                    .first()
                    .map(|record| record.updated_at.0)
                    .unwrap_or_default(),
            )
        });
        groups
    }

    pub(super) fn enforce_bounds(&mut self, app_id: &AppId) {
        while self
            .history
            .iter()
            .filter(|record| record.source.app_id() == app_id)
            .count()
            > MAX_PER_APP
        {
            if let Some(index) = self
                .history
                .iter()
                .position(|record| record.source.app_id() == app_id)
            {
                self.history.remove(index);
            }
        }
        if self.history.len() > MAX_HISTORY {
            self.history.drain(..self.history.len() - MAX_HISTORY);
        }
    }
}

fn restrict_lock_preview(
    policy: LockPreview,
    application_hint: LockScreenVisibility,
) -> LockPreview {
    let hint = match application_hint {
        LockScreenVisibility::Policy | LockScreenVisibility::Show => LockPreview::Show,
        LockScreenVisibility::HideContent => LockPreview::HideContent,
        LockScreenVisibility::Hide => LockPreview::Hide,
    };
    if preview_rank(policy) >= preview_rank(hint) {
        policy
    } else {
        hint
    }
}

fn preview_rank(preview: LockPreview) -> u8 {
    match preview {
        LockPreview::Show => 0,
        LockPreview::HideContent => 1,
        LockPreview::Hide => 2,
    }
}

pub struct Group<'a> {
    pub app_id: &'a AppId,
    pub records: Vec<&'a Notification>,
}

impl fmt::Debug for Group<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Group")
            .field("app_id", &"<redacted>")
            .field(
                "records",
                &format_args!("<{} redacted records>", self.records.len()),
            )
            .finish()
    }
}
