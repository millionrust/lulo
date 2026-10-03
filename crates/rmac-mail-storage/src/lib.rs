//! Per-account mail cache. Callers run this synchronous API on a worker thread.

use rmac_mail_store::{thread_messages, MessageHeader, ThreadForest};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

const SCHEMA_VERSION: i32 = 1;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Sql(rusqlite::Error),
    InvalidBlob,
    UnsupportedSchema(i32),
    StaleUidValidity,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "mail cache I/O: {error}"),
            Self::Sql(error) => write!(f, "mail cache database: {error}"),
            Self::InvalidBlob => write!(f, "mail blob failed its content hash"),
            Self::UnsupportedSchema(version) => {
                write!(f, "unsupported mail schema version {version}")
            }
            Self::StaleUidValidity => {
                write!(f, "mailbox UIDVALIDITY changed; resync before replay")
            }
        }
    }
}

impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}
impl From<rusqlite::Error> for Error {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sql(error)
    }
}
pub type Result<T> = std::result::Result<T, Error>;

pub struct MailStorage {
    connection: Connection,
    blobs: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewMessage<'a> {
    pub mailbox_id: i64,
    pub uid: i64,
    pub message_id: Option<&'a str>,
    pub in_reply_to: Option<&'a str>,
    pub references: &'a [&'a str],
    pub subject: &'a str,
    pub sender: &'a str,
    pub recipients: &'a str,
    pub preview: &'a str,
    pub received_at: i64,
    pub flags: i64,
    pub body: Option<&'a [u8]>,
    /// Plain text extracted by the MIME layer, when a body has been fetched.
    pub body_text: Option<&'a str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageSummary {
    pub id: i64,
    pub mailbox_id: i64,
    pub uid: i64,
    pub subject: String,
    pub sender: String,
    pub preview: String,
    pub flags: i64,
    pub body_hash: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    SetFlags(i64),
    Move(i64),
    Delete,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingChange {
    pub id: i64,
    pub message_id: i64,
    pub mailbox_id: i64,
    pub uid: i64,
    pub uidvalidity: i64,
    pub change: Change,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    pub id: i64,
    pub message_id: i64,
    pub filename: String,
    pub mime_type: String,
    pub blob_hash: String,
}

impl MailStorage {
    /// `data_root` is normally ~/.local/share/lulo/mail. A typed UUID prevents
    /// remote account names from becoming path components.
    pub fn open(data_root: &Path, account: Uuid) -> Result<Self> {
        create_private_dir(data_root)?;
        let account_dir = data_root.join(account.to_string());
        create_private_dir(&account_dir)?;
        let blobs = account_dir.join("blobs");
        create_private_dir(&blobs)?;
        let db_path = account_dir.join("index.sqlite3");
        if !db_path.exists() {
            private_new_file(&db_path)?;
        }
        let connection = Connection::open(&db_path)?;
        set_private_file(&db_path)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        let mut store = Self { connection, blobs };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&mut self) -> Result<()> {
        let version: i32 = self
            .connection
            .pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(Error::UnsupportedSchema(version));
        }
        if version == 0 {
            let tx = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch(include_str!("schema_v1.sql"))?;
            tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            tx.commit()?;
        }
        Ok(())
    }

    pub fn upsert_mailbox(
        &mut self,
        name: &str,
        uidvalidity: i64,
        special_use: Option<&str>,
    ) -> Result<i64> {
        self.connection.execute(
            "INSERT INTO mailboxes(name, uidvalidity, special_use) VALUES (?1, ?2, ?3) \
             ON CONFLICT(name) DO UPDATE SET uidvalidity=excluded.uidvalidity, special_use=excluded.special_use",
            params![name, uidvalidity, special_use],
        )?;
        Ok(self
            .connection
            .query_row("SELECT id FROM mailboxes WHERE name=?1", [name], |row| {
                row.get(0)
            })?)
    }

    /// Write the blob durably before the row. A crash may leave an orphan blob,
    /// but can never leave a committed row pointing to a partial blob.
    pub fn put_message(&mut self, message: &NewMessage<'_>) -> Result<i64> {
        let body_hash = message.body.map(|body| self.write_blob(body)).transpose()?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO messages(mailbox_id, uid, message_id, in_reply_to, subject, sender, recipients, preview, received_at, flags, body_hash, body_text) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12) \
             ON CONFLICT(mailbox_id,uid) DO UPDATE SET message_id=excluded.message_id, in_reply_to=excluded.in_reply_to, \
             subject=excluded.subject, sender=excluded.sender, recipients=excluded.recipients, preview=excluded.preview, \
             received_at=excluded.received_at, flags=excluded.flags, body_hash=COALESCE(excluded.body_hash,messages.body_hash), \
             body_text=COALESCE(excluded.body_text,messages.body_text)",
            params![message.mailbox_id, message.uid, message.message_id, message.in_reply_to,
                message.subject, message.sender, message.recipients, message.preview,
                message.received_at, message.flags, body_hash, message.body_text],
        )?;
        let id = tx.query_row(
            "SELECT id FROM messages WHERE mailbox_id=?1 AND uid=?2",
            params![message.mailbox_id, message.uid],
            |row| row.get(0),
        )?;
        tx.execute("DELETE FROM message_references WHERE message_id=?1", [id])?;
        for (position, reference) in message.references.iter().enumerate() {
            tx.execute(
                "INSERT INTO message_references(message_id,position,reference) VALUES (?1,?2,?3)",
                params![id, position as i64, reference],
            )?;
        }
        tx.commit()?;
        Ok(id)
    }

    /// Compute conversations from cached headers. Message indices in the forest
    /// correspond to the returned IDs, so callers can map back to SQLite rows.
    pub fn threads(&self, mailbox_id: i64) -> Result<(Vec<i64>, ThreadForest)> {
        let mut statement = self.connection.prepare(
            "SELECT id,message_id,in_reply_to,subject FROM messages WHERE mailbox_id=?1 ORDER BY received_at,id")?;
        let rows = statement.query_map([mailbox_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        let mut ids = Vec::new();
        let mut headers = Vec::new();
        let mut references_statement = self.connection.prepare(
            "SELECT reference FROM message_references WHERE message_id=?1 ORDER BY position",
        )?;
        for row in rows {
            let (id, message_id, in_reply_to, subject) = row?;
            let references = references_statement
                .query_map([id], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            ids.push(id);
            headers.push(MessageHeader {
                message_id,
                references,
                in_reply_to,
                subject,
            });
        }
        Ok((ids, thread_messages(&headers)))
    }

    pub fn get_message(&self, id: i64) -> Result<Option<MessageSummary>> {
        Ok(self.connection.query_row(
            "SELECT id,mailbox_id,uid,subject,sender,preview,flags,body_hash FROM messages WHERE id=?1",
            [id], summary_from_row,
        ).optional()?)
    }

    pub fn search(
        &self,
        text: &str,
        mailbox_id: Option<i64>,
        limit: usize,
    ) -> Result<Vec<MessageSummary>> {
        // A quoted phrase treats user input as data rather than FTS5 operators.
        let phrase = format!("\"{}\"", text.replace('"', "\"\""));
        let mut statement = self.connection.prepare(
            "SELECT m.id,m.mailbox_id,m.uid,m.subject,m.sender,m.preview,m.flags,m.body_hash \
             FROM messages_fts JOIN messages m ON m.id=messages_fts.rowid \
             WHERE messages_fts MATCH ?1 AND (?2 IS NULL OR m.mailbox_id=?2) \
             ORDER BY bm25(messages_fts), m.received_at DESC LIMIT ?3",
        )?;
        let rows = statement.query_map(
            params![phrase, mailbox_id, i64::try_from(limit).unwrap_or(i64::MAX)],
            summary_from_row,
        )?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Error::from)
    }

    pub fn write_blob(&self, bytes: &[u8]) -> Result<String> {
        let hash = format!("{:x}", Sha256::digest(bytes));
        let destination = self.blobs.join(&hash);
        if destination.exists() {
            return Ok(hash);
        }
        let temporary = self.blobs.join(format!(".{}-{}.tmp", hash, Uuid::new_v4()));
        let mut file = private_new_file(&temporary)?;
        let result = (|| -> Result<()> {
            file.write_all(bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, &destination)?;
            File::open(&self.blobs)?.sync_all()?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result?;
        Ok(hash)
    }

    pub fn read_blob(&self, hash: &str) -> Result<Vec<u8>> {
        if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(Error::InvalidBlob);
        }
        let mut bytes = Vec::new();
        File::open(self.blobs.join(hash))?.read_to_end(&mut bytes)?;
        if format!("{:x}", Sha256::digest(&bytes)) != hash {
            return Err(Error::InvalidBlob);
        }
        Ok(bytes)
    }

    pub fn put_attachment(
        &mut self,
        message_id: i64,
        filename: &str,
        mime_type: &str,
        bytes: &[u8],
    ) -> Result<i64> {
        let hash = self.write_blob(bytes)?;
        self.connection.execute(
            "INSERT INTO attachments(message_id,filename,mime_type,blob_hash) VALUES (?1,?2,?3,?4)",
            params![message_id, filename, mime_type, hash],
        )?;
        Ok(self.connection.last_insert_rowid())
    }

    pub fn attachments(&self, message_id: i64) -> Result<Vec<Attachment>> {
        let mut statement = self.connection.prepare(
            "SELECT id,message_id,filename,mime_type,blob_hash FROM attachments WHERE message_id=?1 ORDER BY id")?;
        let rows = statement.query_map([message_id], |row| {
            Ok(Attachment {
                id: row.get(0)?,
                message_id: row.get(1)?,
                filename: row.get(2)?,
                mime_type: row.get(3)?,
                blob_hash: row.get(4)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Error::from)
    }

    pub fn queue_change(&mut self, message_id: i64, change: Change) -> Result<i64> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (mailbox_id, uid, uidvalidity): (i64, i64, i64) = tx.query_row(
            "SELECT m.mailbox_id,m.uid,b.uidvalidity FROM messages m JOIN mailboxes b ON b.id=m.mailbox_id WHERE m.id=?1",
            [message_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        let (kind, value) = match change {
            Change::SetFlags(flags) => {
                tx.execute(
                    "UPDATE messages SET flags=?1 WHERE id=?2",
                    params![flags, message_id],
                )?;
                ("flags", Some(flags))
            }
            Change::Move(target) => {
                tx.query_row("SELECT id FROM mailboxes WHERE id=?1", [target], |row| {
                    row.get::<_, i64>(0)
                })?;
                ("move", Some(target))
            }
            Change::Delete => ("delete", None),
        };
        tx.execute("INSERT INTO changes(message_id,mailbox_id,uid,uidvalidity,kind,value) VALUES (?1,?2,?3,?4,?5,?6)",
            params![message_id,mailbox_id,uid,uidvalidity,kind,value])?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(id)
    }

    pub fn pending_changes(&self) -> Result<Vec<PendingChange>> {
        let mut statement = self.connection.prepare(
            "SELECT id,message_id,mailbox_id,uid,uidvalidity,kind,value FROM changes ORDER BY id",
        )?;
        let rows = statement.query_map([], |row| {
            let kind: String = row.get(5)?;
            let value: Option<i64> = row.get(6)?;
            let change = match (kind.as_str(), value) {
                ("flags", Some(value)) => Change::SetFlags(value),
                ("move", Some(value)) => Change::Move(value),
                ("delete", _) => Change::Delete,
                _ => return Err(rusqlite::Error::InvalidQuery),
            };
            Ok(PendingChange {
                id: row.get(0)?,
                message_id: row.get(1)?,
                mailbox_id: row.get(2)?,
                uid: row.get(3)?,
                uidvalidity: row.get(4)?,
                change,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Error::from)
    }

    /// The sync worker must compare the server's current UIDVALIDITY before
    /// replaying. A changed value invalidates old UIDs and leaves the journal intact.
    pub fn acknowledge_change(&mut self, id: i64, observed_uidvalidity: i64) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let expected: i64 =
            tx.query_row("SELECT uidvalidity FROM changes WHERE id=?1", [id], |row| {
                row.get(0)
            })?;
        if expected != observed_uidvalidity {
            return Err(Error::StaleUidValidity);
        }
        tx.execute("DELETE FROM changes WHERE id=?1", [id])?;
        tx.commit()?;
        Ok(())
    }
}

fn summary_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MessageSummary> {
    Ok(MessageSummary {
        id: row.get(0)?,
        mailbox_id: row.get(1)?,
        uid: row.get(2)?,
        subject: row.get(3)?,
        sender: row.get(4)?,
        preview: row.get(5)?,
        flags: row.get(6)?,
        body_hash: row.get(7)?,
    })
}

fn create_private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn set_private_file(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn private_new_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}

#[cfg(test)]
mod tests;
