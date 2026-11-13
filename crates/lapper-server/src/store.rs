//! SQLite への保存。セッションと、セッションごとの追記だけのイベントのログを持つ。
//!
//! イベントは書き換えも削除もしない。訂正も新しいイベントとして追記する（[`lapper_core::Event`]）。

use std::collections::BTreeMap;
use std::path::Path;

use lapper_core::{Config, Event, GateId};
use rusqlite::{Connection, params};

use crate::error::Result;
use crate::types::SessionInfo;

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
    received_at INTEGER NOT NULL, -- サーバーが受け取った UNIX 時刻（ミリ秒）
    event       TEXT    NOT NULL, -- lapper_core::Event の JSON
    PRIMARY KEY (session_id, seq)
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
        conn.execute_batch(SCHEMA)?;
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

    /// イベントをまとめて追記する。`first_seq` は最初のイベントの連番（それまでのイベントの数）。
    pub fn append(
        &mut self,
        session: i64,
        first_seq: usize,
        events: &[Event],
        received_at: i64,
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut insert = tx.prepare(
                "INSERT INTO events (session_id, seq, received_at, event) VALUES (?1, ?2, ?3, ?4)",
            )?;
            for (i, event) in events.iter().enumerate() {
                let seq = (first_seq + i) as i64;
                insert.execute(params![
                    session,
                    seq,
                    received_at,
                    serde_json::to_string(event)?
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
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
