//! Notification policy and state-transition authority.

use super::*;

impl Server {
    pub fn new(history_limit: usize, timeout_policy: TimeoutPolicy) -> Self {
        Self {
            next_id: 1,
            reserved_ids: BTreeSet::new(),
            timeout_policy,
            history_limit,
            active: BTreeMap::new(),
            history: VecDeque::new(),
        }
    }

    /// Prevent allocation from reusing identifiers retained by an external
    /// crash-safe history authority after this in-memory server restarts.
    pub fn reserve_ids(&mut self, ids: impl IntoIterator<Item = NotificationId>) {
        self.reserved_ids.extend(ids);
    }

    pub fn post(
        &mut self,
        request: Request,
        now: Time,
        policy: DeliveryPolicy,
    ) -> Result<PostOutcome, ServerError> {
        request.validate().map_err(ServerError::Invalid)?;
        let replacement = self.replacement_id(&request)?;
        let (id, kind, created_at) = if let Some(id) = replacement {
            let previous = self
                .active
                .get(&id)
                .ok_or(ServerError::UnknownNotification)?;
            if previous.source.app_id() != request.source.app_id() {
                return Err(ServerError::WrongOwner);
            }
            (id, PostKind::Replaced, previous.created_at)
        } else {
            (self.allocate_id()?, PostKind::Added, now)
        };
        let delivery = delivery_for(&request, policy);
        let expires_at = expiry_for(request.timeout, request.priority, now, self.timeout_policy);
        let announce_as_new = kind == PostKind::Added || request.display.show_as_new;
        let notification = Notification {
            id,
            source: request.source,
            content: request.content,
            priority: request.priority,
            default_action: request.default_action,
            actions: request.actions,
            category: request.category,
            sound: request.sound,
            display: request.display,
            delivery,
            banner_visible: delivery.banner,
            created_at,
            updated_at: now,
            expires_at,
            unread: delivery.history,
        };
        self.remove_history(id);
        if delivery.history {
            self.push_history(notification.clone());
        }
        if policy.enabled {
            self.active.insert(id, notification);
        } else {
            self.active.remove(&id);
        }
        Ok(PostOutcome {
            id,
            kind,
            delivery,
            announce_as_new,
        })
    }

    pub fn withdraw(&mut self, app_id: &AppId, id: NotificationId) -> Result<Closed, ServerError> {
        let notification = self
            .active
            .get(&id)
            .ok_or(ServerError::UnknownNotification)?;
        if notification.source.app_id() != app_id {
            return Err(ServerError::WrongOwner);
        }
        self.active.remove(&id);
        self.remove_history(id);
        Ok(Closed {
            id,
            reason: CloseReason::Withdrawn,
        })
    }

    pub fn withdraw_portal(
        &mut self,
        app_id: &AppId,
        external_id: &str,
    ) -> Result<Closed, ServerError> {
        let id = self
            .active
            .iter()
            .find_map(|(id, notification)| match &notification.source {
                Source::Portal {
                    app_id: owner,
                    external_id: candidate,
                } if owner == app_id && candidate == external_id => Some(*id),
                _ => None,
            })
            .ok_or(ServerError::UnknownNotification)?;
        self.withdraw(app_id, id)
    }

    pub fn dismiss(&mut self, id: NotificationId) -> Result<Closed, ServerError> {
        let notification = self
            .active
            .get(&id)
            .ok_or(ServerError::UnknownNotification)?;
        if notification.display.persistent {
            return Err(ServerError::PersistentNotification);
        }
        self.active.remove(&id);
        self.remove_history(id);
        Ok(Closed {
            id,
            reason: CloseReason::Dismissed,
        })
    }

    pub fn expire(&mut self, now: Time) -> Vec<Closed> {
        let expired: Vec<_> = self
            .active
            .iter()
            .filter_map(|(id, notification)| {
                notification
                    .banner_visible
                    .then_some(notification.expires_at)
                    .flatten()
                    .filter(|expiry| expiry.0 <= now.0)
                    .map(|_| *id)
            })
            .collect();
        expired
            .into_iter()
            .filter_map(|id| self.expire_one(id).ok())
            .collect()
    }

    /// Closes one visibly presented banner when the presentation runtime's
    /// exact (possibly hover/focus-paused) deadline elapses.
    pub fn expire_one(&mut self, id: NotificationId) -> Result<Closed, ServerError> {
        let retain = self
            .active
            .get(&id)
            .ok_or(ServerError::UnknownNotification)?
            .delivery
            .history;
        if retain {
            if let Some(notification) = self.active.get_mut(&id) {
                notification.banner_visible = false;
                notification.expires_at = None;
            }
        } else {
            self.active.remove(&id);
        }
        Ok(Closed {
            id,
            reason: CloseReason::Expired,
        })
    }

    pub fn invoke(
        &mut self,
        id: NotificationId,
        action_id: &str,
    ) -> Result<(ActionInvocation, Option<Closed>), ServerError> {
        let notification = self
            .active
            .get(&id)
            .ok_or(ServerError::UnknownNotification)?;
        let index = notification
            .default_action
            .iter()
            .chain(notification.actions.iter())
            .position(|action| action.id() == action_id)
            .ok_or(ServerError::UnknownAction)?;
        self.invoke_index(id, index)
    }

    /// Activates the notification's default action, if declared.
    pub fn invoke_default(
        &mut self,
        id: NotificationId,
    ) -> Result<(ActionInvocation, Option<Closed>), ServerError> {
        if self
            .active
            .get(&id)
            .ok_or(ServerError::UnknownNotification)?
            .default_action
            .is_none()
        {
            return Err(ServerError::UnknownAction);
        }
        self.invoke_index(id, 0)
    }

    /// Activates a portal button by its declared position. This preserves the
    /// target when buttons intentionally export the same action name.
    pub fn invoke_button(
        &mut self,
        id: NotificationId,
        button_index: usize,
    ) -> Result<(ActionInvocation, Option<Closed>), ServerError> {
        let notification = self
            .active
            .get(&id)
            .ok_or(ServerError::UnknownNotification)?;
        if button_index >= notification.actions.len() {
            return Err(ServerError::UnknownAction);
        }
        let index = usize::from(notification.default_action.is_some()) + button_index;
        self.invoke_index(id, index)
    }

    fn invoke_index(
        &mut self,
        id: NotificationId,
        index: usize,
    ) -> Result<(ActionInvocation, Option<Closed>), ServerError> {
        let notification = self
            .active
            .get(&id)
            .ok_or(ServerError::UnknownNotification)?;
        let action = notification
            .default_action
            .iter()
            .chain(notification.actions.iter())
            .nth(index)
            .ok_or(ServerError::UnknownAction)?;
        let invocation = ActionInvocation {
            notification_id: id,
            app_id: notification.source.app_id().clone(),
            action_id: action.id.clone(),
            target: action.target.clone(),
            purpose: action.purpose.clone(),
        };
        let closed = if notification.display.resident {
            None
        } else {
            self.active.remove(&id);
            self.remove_history(id);
            Some(Closed {
                id,
                reason: CloseReason::ActionInvoked,
            })
        };
        Ok((invocation, closed))
    }

    pub fn mark_all_read(&mut self) {
        for notification in &mut self.history {
            notification.unread = false;
        }
        for notification in self.active.values_mut() {
            notification.unread = false;
        }
    }

    pub fn clear_history(&mut self, app_id: Option<&AppId>) {
        self.history.retain(|notification| {
            app_id.is_some_and(|app_id| notification.source.app_id() != app_id)
        });
    }

    pub fn active(&self) -> impl Iterator<Item = &Notification> {
        self.active.values()
    }

    pub fn history(&self) -> impl DoubleEndedIterator<Item = &Notification> {
        self.history.iter()
    }

    pub fn indicator(&self) -> Indicator {
        let unread = self
            .history
            .iter()
            .filter(|notification| notification.unread);
        Indicator {
            unread_count: unread.clone().count().try_into().unwrap_or(u32::MAX),
            has_urgent: unread
                .into_iter()
                .any(|notification| notification.priority == Priority::Urgent),
        }
    }

    fn replacement_id(&self, request: &Request) -> Result<Option<NotificationId>, ServerError> {
        if let Some(id) = request.replaces {
            return Ok(Some(id));
        }
        let Source::Portal {
            app_id,
            external_id,
        } = &request.source
        else {
            return Ok(None);
        };
        Ok(self
            .active
            .iter()
            .find_map(|(id, notification)| match &notification.source {
                Source::Portal {
                    app_id: owner,
                    external_id: candidate,
                } if owner == app_id && candidate == external_id => Some(*id),
                _ => None,
            }))
    }

    fn allocate_id(&mut self) -> Result<NotificationId, ServerError> {
        for _ in 0..u32::MAX {
            let candidate = self.next_id.max(1);
            self.next_id = candidate.wrapping_add(1).max(1);
            let id = NotificationId(candidate);
            if !self.active.contains_key(&id) && !self.reserved_ids.contains(&id) {
                return Ok(id);
            }
        }
        Err(ServerError::ExhaustedIds)
    }

    fn push_history(&mut self, notification: Notification) {
        if self.history_limit == 0 {
            return;
        }
        self.history.push_back(notification);
        while self.history.len() > self.history_limit {
            self.history.pop_front();
        }
    }

    fn remove_history(&mut self, id: NotificationId) {
        self.history.retain(|notification| notification.id != id);
    }
}

pub(super) fn delivery_for(request: &Request, policy: DeliveryPolicy) -> Delivery {
    if !policy.enabled {
        return Delivery {
            banner: false,
            history: false,
            sound: false,
        };
    }
    let focus_allows = !policy.focus_active
        || (policy.allow_urgent_through_focus && request.priority == Priority::Urgent);
    let banner = !request.display.tray_only && policy.banner == BannerPolicy::Allow && focus_allows;
    let history = !request.display.transient && policy.history == HistoryPolicy::Allow;
    let sound = focus_allows && policy.sounds && request.sound != Sound::Silent;
    Delivery {
        banner,
        history,
        sound,
    }
}

pub(super) fn expiry_for(
    timeout: Timeout,
    priority: Priority,
    now: Time,
    policy: TimeoutPolicy,
) -> Option<Time> {
    let duration = match timeout {
        Timeout::Never => return None,
        Timeout::Milliseconds(value) => value,
        Timeout::Default => match priority {
            Priority::Low => policy.low_ms,
            Priority::Normal => policy.normal_ms,
            Priority::High => policy.high_ms,
            Priority::Urgent => return None,
        },
    };
    Some(Time(now.0.saturating_add(duration)))
}

pub(super) fn validate_identifier(
    value: &str,
    max: usize,
    field: Field,
) -> Result<(), ValidationError> {
    if value.trim().is_empty() {
        return Err(ValidationError::new(field, Problem::Empty));
    }
    if value.len() > max {
        return Err(ValidationError::new(field, Problem::TooLong));
    }
    if value.chars().any(char::is_control) {
        return Err(ValidationError::new(field, Problem::Invalid));
    }
    Ok(())
}

pub(super) fn validate_text(
    value: &str,
    max: usize,
    multiline: bool,
    field: Field,
) -> Result<(), ValidationError> {
    if value.len() > max {
        return Err(ValidationError::new(field, Problem::TooLong));
    }
    if value
        .chars()
        .any(|character| character.is_control() && !(multiline && matches!(character, '\n' | '\t')))
    {
        return Err(ValidationError::new(field, Problem::Invalid));
    }
    Ok(())
}
