//! Seeds a Notes library for the speed sweep's Notes scenarios
//! (scripts/behavior/speed_interactions.py): `COUNT` short notes plus one
//! long note of about `LONG_BYTES`, the long one most recently edited so
//! Notes opens it.
//!
//!     seed_speed_fixture ROOT COUNT LONG_BYTES
//!
//! ROOT is the Notes data root (`$XDG_DATA_HOME/rmac/notes`). Run it before
//! Notes starts: it takes the library's writer lease.

use std::path::PathBuf;

use rmac_notes_storage::NotesLibraryStore;
use rmac_notes_store::{LibrarySnapshot, NoteId, NoteRecord};

fn main() {
    let mut args = std::env::args().skip(1);
    let usage = "usage: seed_speed_fixture ROOT COUNT LONG_BYTES";
    let root = PathBuf::from(args.next().expect(usage));
    let count: u64 = args.next().and_then(|v| v.parse().ok()).expect(usage);
    let long_bytes: usize = args.next().and_then(|v| v.parse().ok()).expect(usage);

    let store = NotesLibraryStore::new(root).expect("take the Notes writer lease");
    let loaded = store.load().expect("load the Notes library");
    let mut candidate: LibrarySnapshot = loaded.snapshot().clone();
    let base_ms = 1_790_000_000_000_u64;
    let paragraph = "The quick brown fox jumps over the lazy dog while the speed sweep \
                     measures how quickly every keystroke reaches the screen. ";
    for index in 0..count {
        let id = candidate.next_note_id;
        let long = index + 1 == count;
        let body = if long {
            let mut body = String::with_capacity(long_bytes + paragraph.len());
            let mut line = 0;
            while body.len() < long_bytes {
                line += 1;
                body.push_str(&format!("{line}. {paragraph}\n"));
            }
            body
        } else {
            format!("Note {index} body.\n{paragraph}")
        };
        candidate.notes.push(NoteRecord {
            id: NoteId::new(id).expect("non-zero note id"),
            revision: 1,
            created_unix_ms: base_ms + index * 60_000,
            modified_unix_ms: base_ms + index * 60_000,
            title: if long {
                "Long note".to_owned()
            } else {
                format!("Speed note {index:04}")
            },
            body,
            tags: Vec::new(),
            folder_id: None,
            pinned: false,
            deleted: false,
            attachments: Vec::new(),
            lock: None,
        });
        candidate.next_note_id = id + 1;
    }
    candidate.revision = loaded.snapshot().revision + 1;
    store
        .save(&loaded, &candidate)
        .expect("save the seeded Notes library");
    println!("seeded {count} notes");
}
