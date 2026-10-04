-- MAIL-9: Microsoft Graph identities. Graph messages and folders have opaque
-- string ids instead of IMAP UIDs; these tables map them onto the existing
-- (mailbox, uid) rows. `sync_cursor` is Graph's delta or next link for the
-- folder: an opaque sync position, never a credential.
CREATE TABLE IF NOT EXISTS remote_mailboxes (
    mailbox_id INTEGER PRIMARY KEY REFERENCES mailboxes(id) ON DELETE CASCADE,
    remote_id TEXT NOT NULL UNIQUE,
    sync_cursor TEXT
);
CREATE TABLE IF NOT EXISTS remote_messages (
    message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    remote_id TEXT NOT NULL UNIQUE
);
