//! The SQLite file on the mailbox machine. One table of mailboxes (tokens
//! hashed), one of messages. Same pragmas as anathem: WAL and a busy timeout,
//! so a backup or a second process does not turn into a lock error.

use crate::api::{self, Message};
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::time::Duration;

pub struct Store {
    conn: Connection,
}

pub enum InsertOutcome {
    New(String),
    /// The same sender retried the same `client_id`; return the original id.
    Duplicate(String),
    /// The recipient's pending queue is at the cap.
    Full,
}

/// One message on its way into the store. A struct instead of eight
/// positional arguments, which is where `insert` was heading.
pub struct NewMessage<'a> {
    pub id: &'a str,
    pub from: &'a str,
    pub to: &'a str,
    pub client_id: &'a str,
    pub project: &'a str,
    pub body: &'a str,
    pub now: i64,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                let existed = parent.exists();
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("cannot create {}", parent.display()))?;
                if !existed {
                    restrict(parent, 0o700)?;
                }
            }
        }
        let conn =
            Connection::open(path).with_context(|| format!("cannot open {}", path.display()))?;
        restrict(path, 0o600)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.execute_batch(
            "pragma journal_mode = WAL;
             pragma synchronous = NORMAL;
             pragma foreign_keys = ON;",
        )?;
        let store = Self { conn };
        store.init()?;
        Ok(store)
    }

    fn init(&self) -> Result<()> {
        self.conn.execute_batch(
            "create table if not exists mailboxes (
                 name       text primary key,
                 token_hash text not null unique,
                 created_at integer not null
             );
             create table if not exists messages (
                 id          text primary key,
                 from_box    text not null,
                 to_box      text not null,
                 client_id   text not null,
                 project     text not null,
                 body        text not null,
                 created_at  integer not null,
                 expires_at  integer not null,
                 delivered_at integer,
                 unique (from_box, client_id)
             );
             create index if not exists idx_messages_pending
                 on messages (to_box, delivered_at, created_at);
             create index if not exists idx_messages_expiry
                 on messages (expires_at);",
        )?;
        Ok(())
    }

    pub fn add_mailbox(&self, name: &str, token_hash: &str, now: i64) -> Result<()> {
        if self.mailbox_exists(name)? {
            bail!("mailbox {name:?} already exists; use `stele token rotate {name}`");
        }
        self.conn.execute(
            "insert into mailboxes (name, token_hash, created_at) values (?1, ?2, ?3)",
            params![name, token_hash, now],
        )?;
        Ok(())
    }

    pub fn rotate_mailbox(&self, name: &str, token_hash: &str) -> Result<()> {
        let changed = self.conn.execute(
            "update mailboxes set token_hash = ?2 where name = ?1",
            params![name, token_hash],
        )?;
        if changed == 0 {
            bail!("there is no mailbox {name:?}");
        }
        Ok(())
    }

    pub fn remove_mailbox(&self, name: &str) -> Result<bool> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "delete from messages where to_box = ?1 or from_box = ?1",
            [name],
        )?;
        let removed = tx.execute("delete from mailboxes where name = ?1", [name])? > 0;
        tx.commit()?;
        Ok(removed)
    }

    pub fn mailbox_exists(&self, name: &str) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "select 1 from mailboxes where name = ?1",
                [name],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    pub fn mailbox_for_hash(&self, token_hash: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "select name from mailboxes where token_hash = ?1",
                [token_hash],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn mailboxes(&self) -> Result<Vec<String>> {
        let mut st = self
            .conn
            .prepare("select name from mailboxes order by name")?;
        let rows = st.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<String>>>()?)
    }

    pub fn insert(&self, message: NewMessage) -> Result<InsertOutcome> {
        let NewMessage {
            id,
            from,
            to,
            client_id,
            project,
            body,
            now,
        } = message;
        if let Some(existing) = self
            .conn
            .query_row(
                "select id from messages where from_box = ?1 and client_id = ?2",
                params![from, client_id],
                |r| r.get::<_, String>(0),
            )
            .optional()?
        {
            return Ok(InsertOutcome::Duplicate(existing));
        }
        let pending: i64 = self.conn.query_row(
            "select count(*) from messages
              where to_box = ?1 and delivered_at is null and expires_at > ?2",
            params![to, now],
            |r| r.get(0),
        )?;
        if pending >= api::MAX_PENDING {
            return Ok(InsertOutcome::Full);
        }
        self.conn.execute(
            "insert into messages
               (id, from_box, to_box, client_id, project, body, created_at, expires_at)
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                id,
                from,
                to,
                client_id,
                project,
                body,
                now,
                now + api::TTL_SECS
            ],
        )?;
        Ok(InsertOutcome::New(id.to_string()))
    }

    pub fn unread(&self, to: &str, limit: i64, now: i64) -> Result<Vec<Message>> {
        let mut st = self.conn.prepare(
            "select id, from_box, project, body, created_at
               from messages
              where to_box = ?1 and delivered_at is null and expires_at > ?2
              order by created_at, id
              limit ?3",
        )?;
        let rows = st.query_map(params![to, now, limit], |r| {
            Ok(Message {
                id: r.get(0)?,
                from: r.get(1)?,
                project: r.get(2)?,
                body: r.get(3)?,
                created: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Mark a message as handed over. Idempotent: a second ack for the same
    /// message is not an error, because the client may have crashed between
    /// writing the draft and hearing the answer.
    pub fn ack(&self, to: &str, id: &str, now: i64) -> Result<bool> {
        let changed = self.conn.execute(
            "update messages set delivered_at = ?3
              where id = ?1 and to_box = ?2 and delivered_at is null",
            params![id, to, now],
        )?;
        if changed > 0 {
            return Ok(true);
        }
        Ok(self
            .conn
            .query_row(
                "select 1 from messages where id = ?1 and to_box = ?2",
                params![id, to],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    pub fn purge(&self, now: i64) -> Result<usize> {
        Ok(self
            .conn
            .execute("delete from messages where expires_at <= ?1", [now])?)
    }

    pub fn count(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("select count(*) from messages", [], |r| r.get(0))?)
    }
}

/// The mailbox holds message bodies in the clear: keep the file, and the
/// directory it lives in, out of reach of other users — the same stance as
/// anathem's database.
#[cfg(unix)]
fn restrict(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        let current = meta.permissions().mode() & 0o777;
        // Do not loosen a file an operator made stricter on purpose.
        if current & !mode != 0 {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
                .with_context(|| format!("cannot set permissions on {}", path.display()))?;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn restrict(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (Store, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("stele.db")).unwrap();
        store.add_mailbox("dell", "hash-dell", 0).unwrap();
        store.add_mailbox("laptop", "hash-laptop", 0).unwrap();
        (store, dir)
    }

    #[allow(clippy::too_many_arguments)]
    fn add(
        s: &Store,
        id: &str,
        from: &str,
        to: &str,
        client_id: &str,
        project: &str,
        body: &str,
        now: i64,
    ) -> InsertOutcome {
        s.insert(NewMessage {
            id,
            from,
            to,
            client_id,
            project,
            body,
            now,
        })
        .unwrap()
    }

    #[test]
    fn token_lookup() {
        let (s, _d) = store();
        assert_eq!(
            s.mailbox_for_hash("hash-dell").unwrap().as_deref(),
            Some("dell")
        );
        assert_eq!(s.mailbox_for_hash("nope").unwrap(), None);
    }

    #[test]
    fn insert_and_ack() {
        let (s, _d) = store();
        let InsertOutcome::New(id) = add(&s, "m1", "laptop", "dell", "c1", "api", "hello", 1000)
        else {
            panic!("expected a new message");
        };
        assert_eq!(id, "m1");
        let got = s.unread("dell", 50, 1000).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].from, "laptop");
        assert!(s.ack("dell", "m1", 1001).unwrap());
        assert!(s.unread("dell", 50, 1001).unwrap().is_empty());
        assert!(s.ack("dell", "m1", 1002).unwrap(), "ack is idempotent");
        assert!(!s.ack("dell", "missing", 1002).unwrap());
        assert!(!s.ack("laptop", "m1", 1002).unwrap(), "not the recipient");
    }

    #[test]
    fn retries_do_not_duplicate() {
        let (s, _d) = store();
        let a = add(&s, "m1", "laptop", "dell", "same", "api", "hello", 1000);
        assert!(matches!(a, InsertOutcome::New(_)));
        let b = add(&s, "m2", "laptop", "dell", "same", "api", "hello", 1001);
        assert!(matches!(b, InsertOutcome::Duplicate(ref id) if id == "m1"));
        assert_eq!(s.count().unwrap(), 1);
    }

    #[test]
    fn the_queue_has_a_cap() {
        let (s, _d) = store();
        for i in 0..api::MAX_PENDING {
            let outcome = add(
                &s,
                &format!("m{i}"),
                "laptop",
                "dell",
                &format!("c{i}"),
                "api",
                "x",
                1000,
            );
            assert!(matches!(outcome, InsertOutcome::New(_)));
        }
        let outcome = add(&s, "over", "laptop", "dell", "cover", "api", "x", 1000);
        assert!(matches!(outcome, InsertOutcome::Full));
        s.ack("dell", "m0", 1001).unwrap();
        let outcome = add(&s, "fits", "laptop", "dell", "cfits", "api", "x", 1001);
        assert!(matches!(outcome, InsertOutcome::New(_)));
    }

    #[test]
    fn expired_messages_are_not_delivered() {
        let (s, _d) = store();
        let start = 1000;
        add(&s, "m1", "laptop", "dell", "c1", "api", "hello", start);
        assert!(s
            .unread("dell", 50, start + api::TTL_SECS)
            .unwrap()
            .is_empty());
        add(
            &s,
            "m2",
            "laptop",
            "dell",
            "c2",
            "api",
            "hello",
            start + api::TTL_SECS,
        );
        assert_eq!(s.purge(start + api::TTL_SECS).unwrap(), 1);
        let left = s.unread("dell", 50, start + api::TTL_SECS).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id, "m2");
    }

    #[test]
    fn removing_a_mailbox_removes_its_mail() {
        let (s, _d) = store();
        add(&s, "m1", "laptop", "dell", "c1", "api", "hello", 1000);
        assert!(s.remove_mailbox("dell").unwrap());
        assert_eq!(s.count().unwrap(), 0);
        assert!(!s.mailbox_exists("dell").unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn the_database_and_its_new_directory_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("stele");
        let db = nested.join("stele.db");
        let _store = Store::open(&db).unwrap();
        let file_mode = std::fs::metadata(&db).unwrap().permissions().mode() & 0o777;
        let dir_mode = std::fs::metadata(&nested).unwrap().permissions().mode() & 0o777;
        assert_eq!(file_mode, 0o600);
        assert_eq!(dir_mode, 0o700);
    }
}
