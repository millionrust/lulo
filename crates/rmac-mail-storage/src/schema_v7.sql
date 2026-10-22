-- Mailbox ▸ New, Rename and Delete Mailbox…. `delimiter` is the hierarchy
-- separator the server reports (IMAP LIST; "/" for Graph). Each row of
-- `mailbox_changes` is one folder change already applied to this cache and
-- still to be sent to the server, oldest first.
ALTER TABLE mailboxes ADD COLUMN delimiter TEXT;
CREATE TABLE IF NOT EXISTS mailbox_changes (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL CHECK(kind IN ('create','rename','delete')),
    name TEXT NOT NULL,
    new_name TEXT,
    remote_id TEXT,
    parent_remote_id TEXT
);
