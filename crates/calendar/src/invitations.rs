//! Invitation state (CAL-8): reads the ORGANIZER/ATTENDEE lines EDS keeps
//! unparsed in `rmac_calendar_store::Event::other_properties` and builds an
//! Accept/Maybe/Decline reply. `Calendar::send` (CalDAV scheduling, Google
//! and iCloud) delivers the reply to the organiser; other backends return a
//! recipient list Mail would need to send by iMIP (MAIL-8, not yet built --
//! `docs/design/calendar-mail.md` §2 "Invitations").

use chrono::{DateTime, Utc};
use rmac_calendar_store::Event as IcalEvent;

use crate::{Event, WeekSnapshot};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PartStat {
    NeedsAction,
    Accepted,
    Declined,
    Tentative,
    /// Any other IANA or vendor value (`DELEGATED`, `IN-PROCESS`…), kept
    /// as-is rather than misreported as one of the four Calendar shows.
    Other,
}

impl PartStat {
    fn parse(value: &str) -> Self {
        match value.trim().to_ascii_uppercase().as_str() {
            "NEEDS-ACTION" => Self::NeedsAction,
            "ACCEPTED" => Self::Accepted,
            "DECLINED" => Self::Declined,
            "TENTATIVE" => Self::Tentative,
            _ => Self::Other,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::NeedsAction | Self::Other => "NEEDS-ACTION",
            Self::Accepted => "ACCEPTED",
            Self::Declined => "DECLINED",
            Self::Tentative => "TENTATIVE",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Attendee {
    pub email: String,
    pub name: Option<String>,
    pub partstat: PartStat,
}

/// The inbox popover and inspector's three replies (CAL-8; the design doc's
/// "Accept / Maybe / Decline").
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Response {
    Accept,
    Maybe,
    Decline,
}

impl Response {
    pub const ALL: [Self; 3] = [Self::Accept, Self::Maybe, Self::Decline];

    fn partstat(self) -> PartStat {
        match self {
            Self::Accept => PartStat::Accepted,
            Self::Maybe => PartStat::Tentative,
            Self::Decline => PartStat::Declined,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Accept => "Accept",
            Self::Maybe => "Maybe",
            Self::Decline => "Decline",
        }
    }
}

/// One pending RSVP, as the toolbar's inbox popover lists it.
#[derive(Clone, Debug)]
pub struct PendingInvitation {
    pub event_id: String,
    pub calendar: usize,
    pub title: String,
    pub start: DateTime<Utc>,
    pub all_day: bool,
    pub organizer: Option<String>,
}

fn mailto_address(value: &str) -> Option<String> {
    let value = value.trim();
    value
        .strip_prefix("mailto:")
        .or_else(|| value.strip_prefix("MAILTO:"))
        .map(|address| address.trim().to_ascii_lowercase())
        .filter(|address| !address.is_empty())
}

/// One `;KEY=value` parameter from a property's name-and-parameters half
/// ("ATTENDEE;CN=Jane;PARTSTAT=ACCEPTED"), quoted or not.
fn param(name_and_params: &str, key: &str) -> Option<String> {
    name_and_params.split(';').skip(1).find_map(|segment| {
        let (candidate, value) = segment.split_once('=')?;
        candidate
            .eq_ignore_ascii_case(key)
            .then(|| value.trim_matches('"').to_owned())
    })
}

fn property_name(name_and_params: &str) -> &str {
    name_and_params.split(';').next().unwrap_or_default()
}

fn parse_attendee(line: &str) -> Option<Attendee> {
    let (head, value) = line.split_once(':')?;
    if !property_name(head).eq_ignore_ascii_case("ATTENDEE") {
        return None;
    }
    let email = mailto_address(value)?;
    let partstat = param(head, "PARTSTAT")
        .map(|value| PartStat::parse(&value))
        .unwrap_or(PartStat::NeedsAction);
    Some(Attendee {
        email,
        name: param(head, "CN"),
        partstat,
    })
}

/// The event's ORGANIZER, if it has one -- the mark of a meeting rather
/// than a plain personal event.
pub fn organizer(event: &IcalEvent) -> Option<Attendee> {
    event.other_properties.iter().find_map(|line| {
        let (head, value) = line.split_once(':')?;
        if !property_name(head).eq_ignore_ascii_case("ORGANIZER") {
            return None;
        }
        Some(Attendee {
            email: mailto_address(value)?,
            name: param(head, "CN"),
            partstat: PartStat::Accepted,
        })
    })
}

/// Every ATTENDEE on the event, in VEVENT order.
pub fn attendees(event: &IcalEvent) -> Vec<Attendee> {
    event
        .other_properties
        .iter()
        .filter_map(|line| parse_attendee(line.as_str()))
        .collect()
}

/// This computer's own RSVP line, matched case-insensitively against any of
/// `self_emails` (the calendar-enabled signed-in accounts' addresses).
pub fn my_attendee(event: &IcalEvent, self_emails: &[String]) -> Option<Attendee> {
    if self_emails.is_empty() {
        return None;
    }
    attendees(event).into_iter().find(|attendee| {
        self_emails
            .iter()
            .any(|email| email.eq_ignore_ascii_case(&attendee.email))
    })
}

/// True when this computer has an outstanding RSVP on `event`: an
/// organiser-sent meeting where our own ATTENDEE line still needs a reply.
pub fn needs_response(event: &IcalEvent, self_emails: &[String]) -> bool {
    organizer(event).is_some()
        && my_attendee(event, self_emails)
            .is_some_and(|attendee| attendee.partstat == PartStat::NeedsAction)
}

/// Rewrites this computer's own ATTENDEE line to `response`'s PARTSTAT,
/// leaving every other property -- including every other attendee --
/// untouched. `None` when there is no organiser or no attendee line
/// matching `self_email` to update (nothing to reply to).
pub fn apply_response(
    event: &IcalEvent,
    self_email: &str,
    response: Response,
) -> Option<IcalEvent> {
    if organizer(event).is_none() {
        return None;
    }
    let target = self_email.trim().to_ascii_lowercase();
    if target.is_empty() {
        return None;
    }
    let mut updated = event.clone();
    let mut changed = false;
    for line in &mut updated.other_properties {
        let Some((head, value)) = line.split_once(':') else {
            continue;
        };
        if !property_name(head).eq_ignore_ascii_case("ATTENDEE") {
            continue;
        }
        let Some(email) = mailto_address(value) else {
            continue;
        };
        if email != target {
            continue;
        }
        *line = format!("{}:{value}", set_partstat(head, response.partstat()));
        changed = true;
        break;
    }
    changed.then_some(updated)
}

fn set_partstat(name_and_params: &str, status: PartStat) -> String {
    let mut parts: Vec<String> = name_and_params.split(';').map(str::to_owned).collect();
    let mut found = false;
    for part in parts.iter_mut().skip(1) {
        if let Some((key, _)) = part.split_once('=') {
            if key.eq_ignore_ascii_case("PARTSTAT") {
                *part = format!("PARTSTAT={}", status.as_str());
                found = true;
                break;
            }
        }
    }
    if !found {
        parts.push(format!("PARTSTAT={}", status.as_str()));
    }
    parts.join(";")
}

/// Every event in `snapshot` this computer has an outstanding RSVP for,
/// soonest first, skipping soft-deleted and hidden calendars same as
/// search (CAL-6).
pub fn pending(snapshot: &WeekSnapshot, self_emails: &[String]) -> Vec<PendingInvitation> {
    if self_emails.is_empty() {
        return Vec::new();
    }
    let mut pending: Vec<PendingInvitation> = snapshot
        .events
        .iter()
        .filter_map(|event| pending_invitation(event, snapshot, self_emails))
        .collect();
    pending.sort_by_key(|invitation| invitation.start);
    pending
}

fn pending_invitation(
    event: &Event,
    snapshot: &WeekSnapshot,
    self_emails: &[String],
) -> Option<PendingInvitation> {
    let ical = event.ical.as_ref()?;
    if snapshot
        .calendars
        .get(event.calendar)
        .is_some_and(|calendar| calendar.removed)
    {
        return None;
    }
    if !needs_response(ical, self_emails) {
        return None;
    }
    Some(PendingInvitation {
        event_id: event.id.clone(),
        calendar: event.calendar,
        title: event.title.clone(),
        start: event.start,
        all_day: event.all_day,
        organizer: organizer(ical).map(|organizer| {
            organizer
                .name
                .filter(|name| !name.trim().is_empty())
                .unwrap_or(organizer.email)
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Calendar, CalendarColor};
    use rmac_calendar_store::{TimeValue, Zone};

    fn invitation_ical(partstat: &str) -> IcalEvent {
        IcalEvent {
            uid: "meeting-1".into(),
            summary: "Roadmap review".into(),
            start: TimeValue {
                local: chrono::NaiveDate::from_ymd_opt(2026, 10, 10)
                    .unwrap()
                    .and_hms_opt(10, 0, 0)
                    .unwrap(),
                zone: Zone::Utc,
            },
            end: TimeValue {
                local: chrono::NaiveDate::from_ymd_opt(2026, 10, 10)
                    .unwrap()
                    .and_hms_opt(11, 0, 0)
                    .unwrap(),
                zone: Zone::Utc,
            },
            rrules: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            recurrence_id: None,
            cancelled: false,
            other_properties: vec![
                "ORGANIZER;CN=Ana Reyes:mailto:ana@example.com".into(),
                "ATTENDEE;CN=Ana Reyes;PARTSTAT=ACCEPTED:mailto:ana@example.com".into(),
                format!(
                    "ATTENDEE;CN=Me;ROLE=REQ-PARTICIPANT;PARTSTAT={partstat}:mailto:Me@Example.com"
                ),
            ],
        }
    }

    #[test]
    fn parses_organizer_and_attendees_with_params() {
        let event = invitation_ical("NEEDS-ACTION");
        let organizer = organizer(&event).unwrap();
        assert_eq!(organizer.email, "ana@example.com");
        assert_eq!(organizer.name.as_deref(), Some("Ana Reyes"));
        let attendees = attendees(&event);
        assert_eq!(attendees.len(), 2);
        assert_eq!(attendees[1].email, "me@example.com");
        assert_eq!(attendees[1].partstat, PartStat::NeedsAction);
    }

    #[test]
    fn my_attendee_matches_case_insensitively() {
        let event = invitation_ical("NEEDS-ACTION");
        let mine = my_attendee(&event, &["me@example.com".into()]).unwrap();
        assert_eq!(mine.email, "me@example.com");
        assert!(my_attendee(&event, &["nobody@example.com".into()]).is_none());
        assert!(my_attendee(&event, &[]).is_none());
    }

    #[test]
    fn needs_response_only_for_my_own_needs_action_attendee() {
        let pending_event = invitation_ical("NEEDS-ACTION");
        assert!(needs_response(&pending_event, &["me@example.com".into()]));
        let answered = invitation_ical("ACCEPTED");
        assert!(!needs_response(&answered, &["me@example.com".into()]));
        // A plain event (no ORGANIZER) is never an invitation.
        let mut personal = pending_event.clone();
        personal
            .other_properties
            .retain(|line| !line.starts_with("ORGANIZER"));
        assert!(!needs_response(&personal, &["me@example.com".into()]));
    }

    #[test]
    fn apply_response_rewrites_only_my_attendee_line() {
        let event = invitation_ical("NEEDS-ACTION");
        for (response, expected) in [
            (Response::Accept, PartStat::Accepted),
            (Response::Maybe, PartStat::Tentative),
            (Response::Decline, PartStat::Declined),
        ] {
            let updated = apply_response(&event, "ME@EXAMPLE.COM", response).unwrap();
            let mine = my_attendee(&updated, &["me@example.com".into()]).unwrap();
            assert_eq!(mine.partstat, expected);
            // The organiser's own attendee line is untouched.
            let organizer_line = attendees(&updated)
                .into_iter()
                .find(|attendee| attendee.email == "ana@example.com")
                .unwrap();
            assert_eq!(organizer_line.partstat, PartStat::Accepted);
        }
    }

    #[test]
    fn apply_response_is_none_without_a_matching_attendee_or_organizer() {
        let event = invitation_ical("NEEDS-ACTION");
        assert!(apply_response(&event, "nobody@example.com", Response::Accept).is_none());
        let mut personal = event.clone();
        personal
            .other_properties
            .retain(|line| !line.starts_with("ORGANIZER"));
        assert!(apply_response(&personal, "me@example.com", Response::Accept).is_none());
    }

    fn snapshot_with(event_ical: IcalEvent) -> WeekSnapshot {
        let calendar = Calendar {
            name: "Work".into(),
            account: "Google".into(),
            color: CalendarColor::Blue,
            visible: true,
            source_uid: Some("work-cal".into()),
            writable: true,
            id: "work-cal".into(),
            removed: false,
            subscription_url: None,
        };
        let start = event_ical.start.resolve(chrono_tz::UTC).unwrap();
        let end = event_ical.end.resolve(chrono_tz::UTC).unwrap();
        WeekSnapshot {
            calendars: vec![calendar],
            events: vec![Event {
                id: "0#meeting-1#0".into(),
                title: event_ical.summary.clone(),
                location: String::new(),
                calendar: 0,
                start,
                end,
                all_day: false,
                ical: Some(event_ical),
            }],
            slots: Vec::new(),
        }
    }

    /// The design doc's `calendar/invitation-accept` scenario, exercised as
    /// a fixture (no EDS, no Docker): the invitation starts pending, Accept
    /// updates its PARTSTAT, and it drops out of the inbox popover's list.
    #[test]
    fn invitation_accept_updates_status_and_leaves_the_pending_list() {
        let self_emails = vec!["me@example.com".to_owned()];
        let snapshot = snapshot_with(invitation_ical("NEEDS-ACTION"));
        let before = pending(&snapshot, &self_emails);
        assert_eq!(before.len(), 1);
        assert_eq!(before[0].title, "Roadmap review");
        assert_eq!(before[0].organizer.as_deref(), Some("Ana Reyes"));

        let ical = snapshot.events[0].ical.as_ref().unwrap();
        let updated = apply_response(ical, "me@example.com", Response::Accept).unwrap();
        let mut after_snapshot = snapshot;
        after_snapshot.events[0].ical = Some(updated);
        assert!(pending(&after_snapshot, &self_emails).is_empty());
    }

    #[test]
    fn pending_skips_soft_deleted_calendars_and_requires_self_emails() {
        let snapshot = snapshot_with(invitation_ical("NEEDS-ACTION"));
        assert!(pending(&snapshot, &[]).is_empty());
        let mut removed_calendar = snapshot;
        removed_calendar.calendars[0].removed = true;
        assert!(pending(&removed_calendar, &["me@example.com".into()]).is_empty());
    }
}
