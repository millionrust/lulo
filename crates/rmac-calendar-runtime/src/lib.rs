//! Worker-side calendar state. A UI receives snapshots after EDS signals; it never calls D-Bus.

use rmac_calendar_eds::{Eds, Source, ViewEvent};
use rmac_calendar_store::Calendar as IcalCalendar;
use std::collections::BTreeMap;

pub trait CalendarClient {
    fn online(&self) -> Result<bool, String>;
    fn writable(&self) -> Result<bool, String>;
    fn list(&self, query: &str) -> Result<Vec<String>, String>;
    fn subscribe(&self, query: &str) -> Result<Box<dyn Iterator<Item = ViewEvent>>, String>;
    fn create(&self, objects: &[String]) -> Result<Vec<String>, String>;
    fn modify(&self, objects: &[String], scope: &str) -> Result<(), String>;
    fn remove(&self, ids: &[(String, String)], scope: &str) -> Result<(), String>;
    fn receive(&self, object: &str) -> Result<(), String>;
    fn send(&self, object: &str) -> Result<(Vec<String>, String), String>;
    fn refresh(&self) -> Result<(), String>;
}

pub trait CalendarBackend {
    fn sources(&self) -> Result<Vec<Source>, String>;
    fn open(&self, uid: &str) -> Result<Box<dyn CalendarClient>, String>;
}

impl CalendarBackend for Eds {
    fn sources(&self) -> Result<Vec<Source>, String> {
        Eds::sources(self).map_err(|_| "Calendar service unavailable".into())
    }
    fn open(&self, uid: &str) -> Result<Box<dyn CalendarClient>, String> {
        Eds::open(self, uid)
            .map(|v| Box::new(v) as Box<dyn CalendarClient>)
            .map_err(|_| "Couldn't open calendar".into())
    }
}

impl CalendarClient for rmac_calendar_eds::Calendar {
    fn online(&self) -> Result<bool, String> {
        self.online()
            .map_err(|_| "Calendar status unavailable".into())
    }
    fn writable(&self) -> Result<bool, String> {
        self.writable()
            .map_err(|_| "Calendar status unavailable".into())
    }
    fn list(&self, query: &str) -> Result<Vec<String>, String> {
        self.object_list(query)
            .map_err(|_| "Couldn't load events".into())
    }
    fn subscribe(&self, query: &str) -> Result<Box<dyn Iterator<Item = ViewEvent>>, String> {
        let stream = self
            .view(query)
            .and_then(|view| view.into_events())
            .map_err(|_| "Couldn't watch calendar events".to_owned())?;
        Ok(Box::new(stream))
    }
    fn create(&self, objects: &[String]) -> Result<Vec<String>, String> {
        self.create(objects)
            .map_err(|_| "Couldn't create event".into())
    }
    fn modify(&self, objects: &[String], scope: &str) -> Result<(), String> {
        self.modify(objects, scope)
            .map_err(|_| "Couldn't change event".into())
    }
    fn remove(&self, ids: &[(String, String)], scope: &str) -> Result<(), String> {
        self.remove(ids, scope)
            .map_err(|_| "Couldn't remove event".into())
    }
    fn receive(&self, object: &str) -> Result<(), String> {
        self.receive(object)
            .map_err(|_| "Couldn't receive invitation".into())
    }
    fn send(&self, object: &str) -> Result<(Vec<String>, String), String> {
        self.send(object)
            .map_err(|_| "Couldn't send invitation".into())
    }
    fn refresh(&self) -> Result<(), String> {
        self.refresh()
            .map_err(|_| "Couldn't refresh calendar".into())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub source: Source,
    pub online: bool,
    pub writable: bool,
    pub objects: BTreeMap<String, String>,
    pub initial_load_complete: bool,
}

struct OpenCalendar {
    client: Box<dyn CalendarClient>,
    snapshot: Snapshot,
    events: Box<dyn Iterator<Item = ViewEvent>>,
}

pub struct CalendarRuntime<B> {
    backend: B,
    sources: Vec<Source>,
    open: BTreeMap<String, OpenCalendar>,
}

impl<B: CalendarBackend> CalendarRuntime<B> {
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            sources: Vec::new(),
            open: BTreeMap::new(),
        }
    }

    pub fn reload_sources(&mut self) -> Result<&[Source], String> {
        self.sources = self.backend.sources()?;
        self.open
            .retain(|uid, _| self.sources.iter().any(|s| &s.uid == uid && s.enabled));
        Ok(&self.sources)
    }

    pub fn sources(&self) -> &[Source] {
        &self.sources
    }

    pub fn open(&mut self, uid: &str) -> Result<&Snapshot, String> {
        let source = self
            .sources
            .iter()
            .find(|s| s.uid == uid && s.enabled)
            .ok_or_else(|| "Calendar source unavailable".to_owned())?
            .clone();
        let client = self.backend.open(uid)?;
        let events = client.subscribe("#t")?;
        let mut snapshot = Snapshot {
            source,
            online: client.online()?,
            writable: client.writable()?,
            objects: BTreeMap::new(),
            initial_load_complete: true,
        };
        for raw in client.list("#t")? {
            insert_object(&mut snapshot.objects, raw)?;
        }
        self.open.insert(
            uid.to_owned(),
            OpenCalendar {
                client,
                snapshot,
                events,
            },
        );
        Ok(&self.open[uid].snapshot)
    }

    pub fn snapshot(&self, uid: &str) -> Option<&Snapshot> {
        self.open.get(uid).map(|v| &v.snapshot)
    }

    pub fn apply_view(&mut self, uid: &str, event: ViewEvent) -> Result<&Snapshot, String> {
        let open = self
            .open
            .get_mut(uid)
            .ok_or_else(|| "Calendar is closed".to_owned())?;
        let client = &open.client;
        let snapshot = &mut open.snapshot;
        match event {
            ViewEvent::Added(objects) | ViewEvent::Modified(objects) => {
                for raw in objects {
                    insert_object(&mut snapshot.objects, raw)?;
                }
            }
            ViewEvent::Removed(ids) => {
                for id in ids {
                    snapshot.objects.remove(&id);
                }
            }
            ViewEvent::Complete(result) => {
                result.map_err(|_| "Couldn't load calendar events".to_owned())?;
                snapshot.initial_load_complete = true;
            }
        }
        snapshot.online = client.online()?;
        Ok(snapshot)
    }

    /// Wait for one EDS change on a worker thread; this performs no polling.
    pub fn next_event(&mut self, uid: &str) -> Result<Option<&Snapshot>, String> {
        let event = self
            .open
            .get_mut(uid)
            .ok_or_else(|| "Calendar is closed".to_owned())?
            .events
            .next();
        match event {
            Some(event) => self.apply_view(uid, event).map(Some),
            None => Ok(None),
        }
    }

    pub fn refresh(&mut self, uid: &str) -> Result<&Snapshot, String> {
        let open = self
            .open
            .get_mut(uid)
            .ok_or_else(|| "Calendar is closed".to_owned())?;
        let client = &open.client;
        let snapshot = &mut open.snapshot;
        client.refresh()?;
        snapshot.online = client.online()?;
        Ok(snapshot)
    }

    pub fn create(&self, uid: &str, objects: &[String]) -> Result<Vec<String>, String> {
        self.client(uid)?.create(objects)
    }
    pub fn modify(&self, uid: &str, objects: &[String], scope: &str) -> Result<(), String> {
        self.client(uid)?.modify(objects, scope)
    }
    pub fn remove(&self, uid: &str, ids: &[(String, String)], scope: &str) -> Result<(), String> {
        self.client(uid)?.remove(ids, scope)
    }
    pub fn receive(&self, uid: &str, object: &str) -> Result<(), String> {
        self.client(uid)?.receive(object)
    }
    pub fn send(&self, uid: &str, object: &str) -> Result<(Vec<String>, String), String> {
        self.client(uid)?.send(object)
    }
    fn client(&self, uid: &str) -> Result<&dyn CalendarClient, String> {
        self.open
            .get(uid)
            .map(|v| v.client.as_ref())
            .ok_or_else(|| "Calendar is closed".to_owned())
    }
}

fn insert_object(objects: &mut BTreeMap<String, String>, raw: String) -> Result<(), String> {
    let wrapped = if raw.contains("BEGIN:VCALENDAR") {
        raw.clone()
    } else {
        format!("BEGIN:VCALENDAR\nVERSION:2.0\n{raw}\nEND:VCALENDAR")
    };
    let parsed =
        IcalCalendar::parse(&wrapped).map_err(|_| "Couldn't read calendar event".to_owned())?;
    if parsed.events.len() != 1 {
        return Err("Couldn't read calendar event".into());
    }
    let uid = &parsed.events[0].uid;
    let rid = raw
        .lines()
        .find_map(|line| {
            line.strip_prefix("RECURRENCE-ID").and_then(|value| {
                value
                    .split_once(':')
                    .map(|(_, rid)| rid.trim_end_matches('\r'))
            })
        })
        .unwrap_or("");
    let key = if rid.is_empty() {
        uid.clone()
    } else {
        format!("{uid}\n{rid}")
    };
    objects.insert(key, raw);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    const EVENT: &str = "BEGIN:VEVENT\nUID:a\nSUMMARY:Meeting\nDTSTART:20261003T100000Z\nDTEND:20261003T110000Z\nEND:VEVENT";
    fn source() -> Source {
        Source {
            uid: "local".into(),
            display_name: "Personal".into(),
            parent_uid: None,
            backend: "local".into(),
            enabled: true,
        }
    }
    struct Fake {
        offline: Rc<Cell<bool>>,
    }
    struct FakeClient {
        offline: Rc<Cell<bool>>,
    }
    impl CalendarBackend for Fake {
        fn sources(&self) -> Result<Vec<Source>, String> {
            Ok(vec![source()])
        }
        fn open(&self, _: &str) -> Result<Box<dyn CalendarClient>, String> {
            Ok(Box::new(FakeClient {
                offline: self.offline.clone(),
            }))
        }
    }
    impl CalendarClient for FakeClient {
        fn online(&self) -> Result<bool, String> {
            Ok(!self.offline.get())
        }
        fn writable(&self) -> Result<bool, String> {
            Ok(true)
        }
        fn list(&self, _: &str) -> Result<Vec<String>, String> {
            Ok(vec![EVENT.into()])
        }
        fn subscribe(&self, _: &str) -> Result<Box<dyn Iterator<Item = ViewEvent>>, String> {
            Ok(Box::new(std::iter::once(ViewEvent::Removed(vec!["a".into()]))))
        }
        fn create(&self, _: &[String]) -> Result<Vec<String>, String> {
            Ok(vec!["b".into()])
        }
        fn modify(&self, _: &[String], _: &str) -> Result<(), String> {
            Ok(())
        }
        fn remove(&self, _: &[(String, String)], _: &str) -> Result<(), String> {
            Ok(())
        }
        fn receive(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn send(&self, _: &str) -> Result<(Vec<String>, String), String> {
            Ok((vec![], EVENT.into()))
        }
        fn refresh(&self) -> Result<(), String> {
            Ok(())
        }
    }
    #[test]
    fn initial_load_and_view_changes() {
        let offline = Rc::new(Cell::new(false));
        let mut runtime = CalendarRuntime::new(Fake {
            offline: offline.clone(),
        });
        runtime.reload_sources().unwrap();
        assert_eq!(runtime.open("local").unwrap().objects.len(), 1);
        runtime.next_event("local").unwrap().unwrap();
        assert!(runtime.snapshot("local").unwrap().objects.is_empty());
        runtime
            .apply_view("local", ViewEvent::Added(vec![EVENT.into()]))
            .unwrap();
        assert_eq!(runtime.snapshot("local").unwrap().objects.len(), 1);
        offline.set(true);
        assert!(!runtime.refresh("local").unwrap().online);
        assert_eq!(runtime.create("local", &[EVENT.into()]).unwrap(), vec!["b"]);
    }
    #[test]
    fn rejects_missing_source_and_malformed_event() {
        let mut runtime = CalendarRuntime::new(Fake {
            offline: Rc::new(Cell::new(false)),
        });
        runtime.reload_sources().unwrap();
        assert!(runtime.open("missing").is_err());
        runtime.open("local").unwrap();
        assert!(runtime
            .apply_view("local", ViewEvent::Added(vec!["bad".into()]))
            .is_err());
    }
}
