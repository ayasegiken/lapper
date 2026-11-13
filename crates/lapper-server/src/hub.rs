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
use crate::types::{AppendEvents, Appended, NewSession, SessionInfo, Snapshot, default_gate_names};

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

    /// イベントを追記して結果を計算し直す。届き済みの番号のイベント（再送）は無視する。
    pub fn append(&self, id: i64, append: AppendEvents) -> Result<Appended> {
        let AppendEvents { source, mut events } = append;
        if source.trim().is_empty() {
            return Err(Error::BadRequest("source is empty".into()));
        }
        let session = self.get(id)?;
        let mut state = session.state.lock().unwrap();
        let now = state.now();
        for numbered in &mut events {
            if numbered.at_now {
                let at = at_mut(&mut numbered.event)
                    .ok_or_else(|| Error::BadRequest("at_now is for events with `at`".into()))?;
                *at = now;
            }
        }
        // 保存に成功したものだけをメモリーに入れる（保存とメモリーの内容をずらさない）
        let accepted = self.store.lock().unwrap().append(
            id,
            state.log.len(),
            &source,
            events,
            unix_millis(),
        )?;
        let count = accepted.len();
        if count > 0 {
            state.log.extend(accepted);
            session.publish(&state);
        }
        Ok(Appended {
            accepted: count,
            events: state.log.len(),
        })
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

impl State {
    /// 運営の操作を「今」の操作として記録するときの時刻。
    ///
    /// コースの時計の時刻か、届いているどの通過記録よりも後の時刻の、遅いほう。
    /// 同じ時刻だと列の操作が通過より先に効くので、通過記録の 1 µs 後にする（すでに始まった走行を変えないように）。
    fn now(&self) -> Micros {
        let after_passages = self
            .log
            .iter()
            .filter_map(|event| match event {
                Event::Passage(passage) => Some(passage.time + 1),
                _ => None,
            })
            .max();
        // Option の比較では None < Some なので、片方しかなければそちらになる
        self.clock.max(after_passages).unwrap_or(0)
    }
}

/// 時刻（`at`）を持つイベントなら、その時刻への可変参照を返す。
fn at_mut(event: &mut Event) -> Option<&mut Micros> {
    match event {
        Event::Enqueue { at, .. } | Event::Dequeue { at, .. } | Event::ReportOff { at, .. } => {
            Some(at)
        }
        _ => None,
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
    use crate::types::NumberedEvent;

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

    /// 送り手 `test` のイベント。番号は `first` から順に付ける。
    fn numbered(first: u64, events: Vec<Event>) -> AppendEvents {
        AppendEvents {
            source: "test".into(),
            events: events
                .into_iter()
                .zip(first..)
                .map(|(event, seq)| NumberedEvent {
                    seq,
                    event,
                    at_now: false,
                })
                .collect(),
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
        let appended = hub.append(session.id, numbered(0, events)).unwrap();
        assert_eq!((appended.accepted, appended.events), (2, 2));
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
        hub.append(id, numbered(0, vec![pass(1, S, 10 * SECOND)]))
            .unwrap();
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
            hub.append(id, numbered(0, vec![pass(1, S, 10 * SECOND)]))
                .unwrap();
            hub.append(id, numbered(1, vec![pass(2, G, 70 * SECOND)]))
                .unwrap();
            id
        };
        let hub = Hub::open(Store::open(&path).unwrap()).unwrap();
        let snapshot = hub.snapshot(id).unwrap();
        assert_eq!(snapshot.events, 2);
        assert_eq!(
            snapshot.outcome.runs[0].result,
            RunResult::Time(60 * SECOND)
        );
        // 再起動の前に届いた番号は、再起動の後も届き済みとして扱う
        let appended = hub
            .append(id, numbered(1, vec![pass(2, G, 70 * SECOND)]))
            .unwrap();
        assert_eq!((appended.accepted, appended.events), (0, 2));
        drop(hub);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }

    #[test]
    fn unknown_session_is_not_found() {
        let hub = Hub::open(Store::open_in_memory().unwrap()).unwrap();
        assert!(matches!(
            hub.append(42, numbered(0, vec![])),
            Err(Error::NotFound)
        ));
    }

    #[test]
    fn resent_events_are_ignored() {
        let hub = Hub::open(Store::open_in_memory().unwrap()).unwrap();
        let id = hub.create_session(new_session()).unwrap().id;
        let first = vec![pass(1, S, 10 * SECOND), pass(2, G, 70 * SECOND)];
        let appended = hub.append(id, numbered(0, first)).unwrap();
        assert_eq!((appended.accepted, appended.events), (2, 2));

        // 番号 1 は届き済みなので、番号 2 だけを追記する
        let mut rx = hub.subscribe(id).unwrap();
        rx.mark_unchanged();
        let resent = vec![pass(2, G, 70 * SECOND), pass(3, S, 80 * SECOND)];
        let appended = hub.append(id, numbered(1, resent)).unwrap();
        assert_eq!((appended.accepted, appended.events), (1, 3));
        assert!(rx.has_changed().unwrap());

        // すべて届き済みなら、何も変わらず配りもしない
        rx.mark_unchanged();
        let appended = hub
            .append(id, numbered(2, vec![pass(3, S, 80 * SECOND)]))
            .unwrap();
        assert_eq!((appended.accepted, appended.events), (0, 3));
        assert!(!rx.has_changed().unwrap());

        // 送り手が違えば、同じ番号でも別のイベント
        let mut other = numbered(0, vec![pass(4, G, 140 * SECOND)]);
        other.source = "other".into();
        let appended = hub.append(id, other).unwrap();
        assert_eq!((appended.accepted, appended.events), (1, 4));
    }

    #[test]
    fn bad_sources_and_numbers_are_rejected() {
        let hub = Hub::open(Store::open_in_memory().unwrap()).unwrap();
        let id = hub.create_session(new_session()).unwrap().id;
        let mut blank = numbered(0, vec![pass(1, S, 10 * SECOND)]);
        blank.source = " ".into();
        assert!(matches!(hub.append(id, blank), Err(Error::BadRequest(_))));
        let too_large = numbered(i64::MAX as u64 + 1, vec![pass(1, S, 10 * SECOND)]);
        assert!(matches!(
            hub.append(id, too_large),
            Err(Error::BadRequest(_))
        ));
        assert_eq!(hub.snapshot(id).unwrap().events, 0);
    }

    #[test]
    fn operations_now_are_stamped_after_everything_received() {
        let hub = Hub::open(Store::open_in_memory().unwrap()).unwrap();
        let id = hub.create_session(new_session()).unwrap().id;
        let enqueue = |car| Event::Enqueue {
            car: CarId(car),
            at: 0,
        };
        let mut now = numbered(0, vec![enqueue(7)]);
        now.events[0].at_now = true;
        // まだ何も届いていなければ 0
        hub.append(id, now).unwrap();

        hub.append(id, numbered(1, vec![pass(1, S, 10 * SECOND)]))
            .unwrap();
        hub.set_clock(id, 9 * SECOND).unwrap();
        let mut now = numbered(2, vec![enqueue(8)]);
        now.events[0].at_now = true;
        hub.append(id, now).unwrap();
        // 時計より通過記録のほうが新しければ、その 1 µs 後。すでに始まった #7 の走行は変わらない
        let session = hub.get(id).unwrap();
        let state = session.state.lock().unwrap();
        assert_eq!(
            state.log[2],
            Event::Enqueue {
                car: CarId(8),
                at: 10 * SECOND + 1
            }
        );
        assert_eq!(state.log[0], enqueue(7));
        drop(state);
        let snapshot = hub.snapshot(id).unwrap();
        assert_eq!(snapshot.outcome.runs[0].car, Some(CarId(7)));
        assert_eq!(snapshot.outcome.queue, vec![CarId(8)]);

        hub.set_clock(id, 30 * SECOND).unwrap();
        let mut now = numbered(
            3,
            vec![Event::Dequeue {
                car: CarId(8),
                at: 0,
            }],
        );
        now.events[0].at_now = true;
        hub.append(id, now).unwrap();
        assert!(hub.snapshot(id).unwrap().outcome.queue.is_empty());

        // 時刻を持たないイベントには使えない
        let mut bad = numbered(
            4,
            vec![Event::SetVoid {
                passage: PassageId(1),
                void: true,
            }],
        );
        bad.events[0].at_now = true;
        assert!(matches!(hub.append(id, bad), Err(Error::BadRequest(_))));
    }
}
