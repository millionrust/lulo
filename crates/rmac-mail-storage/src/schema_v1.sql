CREATE TABLE mailboxes (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    uidvalidity INTEGER NOT NULL CHECK(uidvalidity >= 0),
    special_use TEXT
);
CREATE TABLE messages (
    id INTEGER PRIMARY KEY,
    mailbox_id INTEGER NOT NULL REFERENCES mailboxes(id) ON DELETE CASCADE,
    uid INTEGER NOT NULL CHECK(uid > 0),
    message_id TEXT,
    in_reply_to TEXT,
    subject TEXT NOT NULL,
    sender TEXT NOT NULL,
    recipients TEXT NOT NULL,
    preview TEXT NOT NULL,
    received_at INTEGER NOT NULL,
    flags INTEGER NOT NULL DEFAULT 0,
    body_hash TEXT,
    body_text TEXT,
    UNIQUE(mailbox_id, uid)
);
CREATE INDEX messages_by_mailbox_date ON messages(mailbox_id, received_at DESC);
CREATE TABLE message_references (
    message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    reference TEXT NOT NULL,
    PRIMARY KEY(message_id,position)
);
CREATE TABLE attachments (
    id INTEGER PRIMARY KEY,
    message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    filename TEXT NOT NULL,
    mime_type TEXT NOT NULL,
    blob_hash TEXT NOT NULL
);
CREATE TABLE changes (
    id INTEGER PRIMARY KEY,
    message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    mailbox_id INTEGER NOT NULL,
    uid INTEGER NOT NULL,
    uidvalidity INTEGER NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('flags','move','delete')),
    value INTEGER
);
CREATE VIRTUAL TABLE messages_fts USING fts5(subject, sender, recipients, preview, body_text);
CREATE TRIGGER messages_fts_insert AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts(rowid,subject,sender,recipients,preview,body_text)
    VALUES (new.id,new.subject,new.sender,new.recipients,new.preview,new.body_text);
END;
CREATE TRIGGER messages_fts_update AFTER UPDATE ON messages BEGIN
    DELETE FROM messages_fts WHERE rowid=old.id;
    INSERT INTO messages_fts(rowid,subject,sender,recipients,preview,body_text)
    VALUES (new.id,new.subject,new.sender,new.recipients,new.preview,new.body_text);
END;
CREATE TRIGGER messages_fts_delete AFTER DELETE ON messages BEGIN
    DELETE FROM messages_fts WHERE rowid=old.id;
END;
