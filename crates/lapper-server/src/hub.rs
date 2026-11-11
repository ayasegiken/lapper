//! メモリー上のセッションと、結果の計算・配信。
//!
//! イベントを追記するか、コースの時計が進むたびに、ログ全体から結果を計算し直す。
//! 結果が変わったときだけ、購読している画面（SSE）に新しい [`Snapshot`] を配る。

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use lapper_core::{Event, Micros, compute};
use tokio::sync::watch;

use crate::error::{Error, Result};
use crate::store::Store;
use crate::types::{NewSession, SessionInfo, Snapshot, default_gate_names};

pub struct Hub {
    store: Mutex<Store>,
    sessions: RwLock<BTreeMap<i64, Arc<Session>>>,
}

struct Session {
    info: SessionInfo,
    state: Mutex<State>,
    /// 最新の計算結果。画面はこれを購読する。
    snapshot: watch::Sender<Arc<Snapshot>>,
}

struct State {
    log: Vec<Event>,
    /// コースの時計の、最後に知らされた時刻。
    clock: Option<Micros>,
}

impl Hub {
    /// 保存されているセッションを読み込み、それぞれの結果を計算する。
    pub fn open(store: Store) -> Result<Hub> {
        let mut sessions = BTreeMap::new();
        for (info, log) in store.load()? {
            sessions.insert(info.id, Arc::new(Session::new(info, log)));
        }
        Ok(Hub {
            store: Mutex::new(store),
            sessions: RwLock::new(sessions),
        })
    }

    pub fn create_session(&self, new: NewSession) -> Result<SessionInfo> {
        if new.name.trim().is_empty() {
            return Err(Error::BadRequest("session name is empty".into()));
        }
        let gate_names = new
            .gate_names
            .unwrap_or_else(|| default_gate_names(&new.config.course));
        let created_at = unix_millis();
        let id = self.store.lock().unwrap().create_session(
            &new.name,
            &new.config,
            &gate_names,
            created_at,
        )?;
        let info = SessionInfo {
            id,
            name: new.name,
            config: new.config,
            gate_names,
            created_at,
        };
        let session = Arc::new(Session::new(info.clone(), Vec::new()));
        self.sessions.write().unwrap().insert(id, session);
        Ok(info)
    }

    /// セッションの一覧。新しい順。
    pub fn sessions(&self) -> Vec<SessionInfo> {
        let sessions = self.sessions.read().unwrap();
        sessions.values().rev().map(|s| s.info.clone()).collect()
    }

    pub fn session(&self, id: i64) -> Result<SessionInfo> {
        Ok(self.get(id)?.info.clone())
    }

    /// イベントを追記して結果を計算し直す。追記後のイベントの数を返す。
    pub fn append(&self, id: i64, events: Vec<Event>) -> Result<usize> {
        let session = self.get(id)?;
        let mut state = session.state.lock().unwrap();
        // 保存に成功したものだけをメモリーに入れる（保存とメモリーの内容をずらさない）
        self.store
            .lock()
            .unwrap()
            .append(id, state.log.len(), &events, unix_millis())?;
        state.log.extend(events);
        session.publish(&state);
        Ok(state.log.len())
    }

    /// コースの時計の時刻を知らせる。時刻は戻らない（古い時刻は無視する）。
    pub fn set_clock(&self, id: i64, time: Micros) -> Result<()> {
        let session = self.get(id)?;
        let mut state = session.state.lock().unwrap();
        if state.clock.is_some_and(|clock| time <= clock) {
            return Ok(());
        }
        state.clock = Some(time);
        session.publish(&state);
        Ok(())
    }

    pub fn snapshot(&self, id: i64) -> Result<Arc<Snapshot>> {
        Ok(self.get(id)?.snapshot.borrow().clone())
    }

    /// 結果の購読。最新の結果を持ち、変わるたびに通知される。
    pub fn subscribe(&self, id: i64) -> Result<watch::Receiver<Arc<Snapshot>>> {
        Ok(self.get(id)?.snapshot.subscribe())
    }

    fn get(&self, id: i64) -> Result<Arc<Session>> {
        let sessions = self.sessions.read().unwrap();
        sessions.get(&id).cloned().ok_or(Error::NotFound)
    }
}

impl Session {
    fn new(info: SessionInfo, log: Vec<Event>) -> Session {
        let state = State { log, clock: None };
        let snapshot = Arc::new(calculate(&info, &state));
        Session {
            info,
            state: Mutex::new(state),
            snapshot: watch::Sender::new(snapshot),
        }
    }

    /// 結果を計算し直し、前と変わっていれば配る。
    fn publish(&self, state: &State) {
        let next = calculate(&self.info, state);
        self.snapshot.send_if_modified(|current| {
            if **current == next {
                return false;
            }
            *current = Arc::new(next);
            true
        });
    }
}

fn calculate(info: &SessionInfo, state: &State) -> Snapshot {
    Snapshot {
        session: info.id,
        events: state.log.len(),
        outcome: compute(&info.config, &state.log, state.clock),
    }
}

fn unix_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use lapper_core::*;

    use super::*;

    const S: GateId = GateId(0);
    const G: GateId = GateId(1);

    fn new_session() -> NewSession {
        let mut config = Config::new(Course::new(vec![S, G]).unwrap());
        config.max_run_time = Some(120 * SECOND);
        NewSession {
            name: "test".into(),
            config,
            gate_names: None,
        }
    }

    fn pass(id: u64, gate: GateId, time: Micros) -> Event {
        Event::Passage(Passage {
            id: PassageId(id),
            gate,
            time,
            source: PassageSource::Sensor,
        })
    }

    #[test]
    fn appended_events_update_the_result() {
        let hub = Hub::open(Store::open_in_memory().unwrap()).unwrap();
        let session = hub.create_session(new_session()).unwrap();
        assert_eq!(session.gate_names[&S], "S");
        let mut rx = hub.subscribe(session.id).unwrap();
        rx.mark_unchanged();

        let events = vec![
            Event::Enqueue {
                car: CarId(7),
                at: 0,
            },
            pass(1, S, 10 * SECOND),
        ];
        assert_eq!(hub.append(session.id, events).unwrap(), 2);
        assert!(rx.has_changed().unwrap());
        let snapshot = rx.borrow_and_update().clone();
        assert_eq!(snapshot.events, 2);
        assert_eq!(snapshot.outcome.runs[0].car, Some(CarId(7)));
        assert_eq!(snapshot.outcome.runs[0].status, RunStatus::Running);
    }

    #[test]
    fn clock_times_out_runs_and_publishes_only_changes() {
        let hub = Hub::open(Store::open_in_memory().unwrap()).unwrap();
        let id = hub.create_session(new_session()).unwrap().id;
        hub.append(id, vec![pass(1, S, 10 * SECOND)]).unwrap();
        let mut rx = hub.subscribe(id).unwrap();
        rx.mark_unchanged();

        // 最大走行時間（120 秒）の前は結果が変わらないので配らない
        hub.set_clock(id, 60 * SECOND).unwrap();
        assert!(!rx.has_changed().unwrap());

        hub.set_clock(id, 131 * SECOND).unwrap();
        assert!(rx.has_changed().unwrap());
        let status = rx.borrow().outcome.runs[0].status;
        assert!(matches!(
            status,
            RunStatus::Off {
                cause: OffCause::TimedOut,
                ..
            }
        ));
    }

    #[test]
    fn sessions_are_restored_from_the_store() {
        let path = std::env::temp_dir().join(format!("lapper-hub-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let id = {
            let hub = Hub::open(Store::open(&path).unwrap()).unwrap();
            let id = hub.create_session(new_session()).unwrap().id;
            hub.append(id, vec![pass(1, S, 10 * SECOND)]).unwrap();
            hub.append(id, vec![pass(2, G, 70 * SECOND)]).unwrap();
            id
        };
        let hub = Hub::open(Store::open(&path).unwrap()).unwrap();
        let snapshot = hub.snapshot(id).unwrap();
        assert_eq!(snapshot.events, 2);
        assert_eq!(
            snapshot.outcome.runs[0].result,
            RunResult::Time(60 * SECOND)
        );
        drop(hub);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }

    #[test]
    fn unknown_session_is_not_found() {
        let hub = Hub::open(Store::open_in_memory().unwrap()).unwrap();
        assert!(matches!(hub.append(42, vec![]), Err(Error::NotFound)));
    }
}
