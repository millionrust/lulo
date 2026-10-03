//! Run with tests/behavior/run_private_eds.sh on the reference laptop.
//! This test refuses to use the owner's session bus or home directories.

use rmac_calendar_eds::{Eds, ViewEvent};

#[test]
#[ignore = "requires a private D-Bus session and temporary XDG directories on the laptop"]
fn local_calendar_round_trip() {
    assert_eq!(std::env::var("RMAC_EDS_PRIVATE_BUS").as_deref(), Ok("1"));
    for name in [
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_CACHE_HOME",
        "XDG_STATE_HOME",
        "XDG_RUNTIME_DIR",
    ] {
        let value = std::env::var(name).expect("temporary XDG directory required");
        assert!(
            value.contains("rmac-eds-private-"),
            "refusing owner's XDG directory"
        );
    }
    let eds = Eds::session().unwrap();
    eds.check_available().unwrap();
    let source = eds
        .sources()
        .unwrap()
        .into_iter()
        .find(|s| s.uid == "cal2-local")
        .expect("private local calendar source");
    assert_eq!(source.backend, "local");
    let calendar = eds.open(&source.uid).unwrap();
    assert!(calendar.writable().unwrap());
    let uid = format!("cal2-test-{}", std::process::id());
    let object = format!("BEGIN:VEVENT\r\nUID:{uid}\r\nSUMMARY:Private test\r\nDTSTART:20261003T100000Z\r\nDTEND:20261003T110000Z\r\nEND:VEVENT\r\n");
    let created = calendar.create(std::slice::from_ref(&object)).unwrap();
    assert_eq!(created, vec![uid.clone()]);
    let mut events = calendar.view("#t").unwrap().into_events().unwrap();
    let mut saw_initial = false;
    loop {
        match events.next().expect("view signal") {
            ViewEvent::Added(objects) => {
                saw_initial |= objects.iter().any(|item| item.contains(&uid))
            }
            ViewEvent::Complete(result) => {
                result.unwrap();
                break;
            }
            _ => {}
        }
    }
    assert!(saw_initial);
    let listed = calendar.object_list("#t").unwrap();
    assert!(listed.iter().any(|item| item.contains(&uid)));
    let modified = object.replace("Private test", "Changed private test");
    calendar.modify(&[modified], "all").unwrap();
    assert!(matches!(events.next(), Some(ViewEvent::Modified(_))));
    assert!(calendar
        .object_list("#t")
        .unwrap()
        .iter()
        .any(|item| item.contains("Changed private test")));
    calendar.remove(&[(uid, String::new())], "all").unwrap();
    assert!(matches!(events.next(), Some(ViewEvent::Removed(_))));
    calendar.refresh().unwrap();
    calendar.close().unwrap();
}
