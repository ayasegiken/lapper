//! SQLite への保存。セッションと、セッションごとの追記だけのイベントのログを持つ。
//!
//! イベントは書き換えも削除もしない。訂正も新しいイベントとして追記する（[`lapper_core::Event`]）。

use std::collections::BTreeMap;
use std::path::Path;

use lapper_core::{Config, Event, GateId};
use rusqlite::{Connection, params};

use crate::error::{Error, Result};
use crate::types::{NumberedEvent, SessionInfo};

/// データベースの形式の版（`PRAGMA user_version`）。形式を変えたら上げる。
pub const VERSION: i64 = 1;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS sessions (
    id         INTEGER PRIMARY KEY,
    name       TEXT    NOT NULL,
    config     TEXT    NOT NULL, -- lapper_core::Config の JSON
    gate_names TEXT    NOT NULL, -- ゲートの表示名の JSON
    created_at INTEGER NOT NULL  -- UNIX 時刻（ミリ秒）
);
CREATE TABLE IF NOT EXISTS events (
    session_id  INTEGER NOT NULL REFERENCES sessions (id),
    seq         INTEGER NOT NULL, -- セッションの中での連番（0 から）
    source      TEXT    NOT NULL, -- 送り手の ID
    source_seq  INTEGER NOT NULL, -- 送り手が付けた番号
    received_at INTEGER NOT NULL, -- サーバーが受け取った UNIX 時刻（ミリ秒）
    event       TEXT    NOT NULL, -- lapper_core::Event の JSON
    PRIMARY KEY (session_id, seq),
    UNIQUE (session_id, source, source_seq)
);
";

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        let conn = Connection::open(path)?;
        // 書き込み中に電源が落ちても、確定した追記は失われないようにする
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))?;
        Store::init(conn)
    }

    /// テスト用。メモリー上のデータベースを使う。
    pub fn open_in_memory() -> Result<Store> {
        Store::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Store> {
        conn.pragma_update(None, "foreign_keys", true)?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version == 0 {
            // 版が付いていないのに表があるなら、版を付ける前の形式
            let tables: i64 =
                conn.query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get(0))?;
            if tables > 0 {
                return Err(Error::DbVersion(0));
            }
            conn.execute_batch(SCHEMA)?;
            conn.pragma_update(None, "user_version", VERSION)?;
        } else if version != VERSION {
            return Err(Error::DbVersion(version));
        }
        Ok(Store { conn })
    }

    /// セッションを作り、その ID を返す。
    pub fn create_session(
        &self,
        name: &str,
        config: &Config,
        gate_names: &BTreeMap<GateId, String>,
        created_at: i64,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO sessions (name, config, gate_names, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![
                name,
                serde_json::to_string(config)?,
                serde_json::to_string(gate_names)?,
                created_at
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// 送り手 `source` のイベントをまとめて追記し、追記したイベントを返す。
    /// 送り手と番号の組がすでにあるイベント（再送）は飛ばす。
    ///
    /// `first_seq` は、最初に追記するイベントのセッションの中での連番（それまでのイベントの数）。
    pub fn append(
        &mut self,
        session: i64,
        first_seq: usize,
        source: &str,
        events: Vec<NumberedEvent>,
        received_at: i64,
    ) -> Result<Vec<Event>> {
        let tx = self.conn.transaction()?;
        let mut accepted = Vec::new();
        {
            // 主キー（連番）の衝突は不具合なのでエラーにし、送り手と番号の衝突だけを飛ばす
            let mut insert = tx.prepare(
                "INSERT INTO events (session_id, seq, source, source_seq, received_at, event)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (session_id, source, source_seq) DO NOTHING",
            )?;
            for NumberedEvent { seq, event, .. } in events {
                // SQLite の整数は i64 なので、それを超える番号は保存できない（追記はすべて取り消す）
                let seq = i64::try_from(seq)
                    .map_err(|_| Error::BadRequest(format!("seq is too large: {seq}")))?;
                let inserted = insert.execute(params![
                    session,
                    (first_seq + accepted.len()) as i64,
                    source,
                    seq,
                    received_at,
                    serde_json::to_string(&event)?
                ])?;
                if inserted > 0 {
                    accepted.push(event);
                }
            }
        }
        tx.commit()?;
        Ok(accepted)
    }

    /// すべてのセッションを、イベントのログと一緒に読み込む。作った順。
    pub fn load(&self) -> Result<Vec<(SessionInfo, Vec<Event>)>> {
        let mut sessions = Vec::new();
        let mut select = self
            .conn
            .prepare("SELECT id, name, config, gate_names, created_at FROM sessions ORDER BY id")?;
        let rows = select.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })?;
        for row in rows {
            let (id, name, config, gate_names, created_at) = row?;
            let info = SessionInfo {
                id,
                name,
                config: serde_json::from_str(&config)?,
                gate_names: serde_json::from_str(&gate_names)?,
                created_at,
            };
            let events = self.events(id)?;
            sessions.push((info, events));
        }
        Ok(sessions)
    }

    fn events(&self, session: i64) -> Result<Vec<Event>> {
        let mut select = self
            .conn
            .prepare("SELECT event FROM events WHERE session_id = ?1 ORDER BY seq")?;
        let rows = select.query_map([session], |row| row.get::<_, String>(0))?;
        let mut events = Vec::new();
        for row in rows {
            events.push(serde_json::from_str(&row?)?);
        }
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn databases_of_other_versions_are_rejected() {
        // 版を付ける前の形式
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE sessions (id INTEGER PRIMARY KEY)")
            .unwrap();
        assert!(matches!(Store::init(conn), Err(Error::DbVersion(0))));

        // 新しい版
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", VERSION + 1)
            .unwrap();
        assert!(matches!(Store::init(conn), Err(Error::DbVersion(v)) if v == VERSION + 1));
    }

    #[test]
    fn new_databases_get_the_current_version() {
        let store = Store::open_in_memory().unwrap();
        let version: i64 = store
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, VERSION);
    }
}
