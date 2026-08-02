use super::*;

pub(crate) fn fixture() -> LibrarySnapshot {
    let note_id = NoteId::new(1).unwrap();
    let folder_id = FolderId::new(1).unwrap();
    let attachment_id = AttachmentId::new(1).unwrap();
    LibrarySnapshot {
        revision: 4,
        sort_order: SortOrder::Title,
        next_note_id: 2,
        next_folder_id: 2,
        next_attachment_id: 2,
        folders: vec![FolderRecord {
            id: folder_id,
            revision: 1,
            name: "Projects".into(),
            deleted: false,
        }],
        notes: vec![NoteRecord {
            id: note_id,
            revision: 3,
            created_unix_ms: 10,
            modified_unix_ms: 20,
            title: "Roadmap".into(),
            body: "- [ ] transaction store\nनमस्ते".into(),
            tags: vec!["rmac".into(), "Planning".into()],
            folder_id: Some(folder_id),
            pinned: true,
            deleted: false,
            attachments: vec![attachment_id],
        }],
        attachments: vec![AttachmentRecord {
            id: attachment_id,
            revision: 1,
            note_id,
            display_name: "diagram.png".into(),
            kind: AttachmentKind::Png,
            byte_len: 2048,
            sha256: [7; 32],
            deleted: false,
        }],
    }
}

#[test]
fn stable_ids_are_nonzero_and_path_independent() {
    assert_eq!(NoteId::new(0), None);
    assert_eq!(NoteId::new(42).unwrap().get(), 42);
    assert!(fixture().validate().is_ok());
}

#[test]
fn duplicate_and_dangling_identities_fail_closed() {
    let mut duplicate = fixture();
    duplicate.notes.push(duplicate.notes[0].clone());
    assert_eq!(duplicate.validate(), Err(ValidationError::DuplicateId));

    let mut dangling = fixture();
    dangling.notes[0].folder_id = Some(FolderId::new(99).unwrap());
    assert_eq!(dangling.validate(), Err(ValidationError::MissingReference));
}

#[test]
fn attachment_ownership_and_deleted_state_are_exact() {
    let mut wrong_owner = fixture();
    wrong_owner.attachments[0].note_id = NoteId::new(2).unwrap();
    assert_eq!(
        wrong_owner.validate(),
        Err(ValidationError::InconsistentAttachment)
    );

    let mut pinned_trash = fixture();
    pinned_trash.notes[0].deleted = true;
    assert_eq!(
        pinned_trash.validate(),
        Err(ValidationError::InvalidDeletedState)
    );
}

#[test]
fn names_tags_text_and_sequences_are_bounded() {
    let mut invalid = fixture();
    invalid.folders[0].name = "../escape".into();
    assert_eq!(invalid.validate(), Err(ValidationError::InvalidName));

    invalid.folders[0].name = " bad ".into();
    assert_eq!(invalid.validate(), Err(ValidationError::InvalidName));

    let mut duplicate_tag = fixture();
    duplicate_tag.notes[0].tags.push("RMAC".into());
    assert_eq!(
        duplicate_tag.validate(),
        Err(ValidationError::DuplicateName)
    );

    let mut sequence = fixture();
    sequence.next_note_id = 1;
    assert_eq!(sequence.validate(), Err(ValidationError::InvalidNextId));
}
