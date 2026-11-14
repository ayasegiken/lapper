//! メモリー上のセッションと、結果の計算・配信。
//!
//! イベントを追記するか、コースの時計が進むたびに、ログ全体から結果を計算し直す。
//! 結果が変わったときだけ、購読している画面（SSE）に新しい [`Snapshot`] を配る。

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use lapper_core::{Event, MILLISECOND, Micros, PassageId, compute};
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
    /// コースの時計の、最後に分かった時刻。
    clock: Option<Clock>,
}

/// コースの時計のある時刻と、それが分かったときのサーバーの時刻。
///
/// 次の通知までの間も、サーバーの時計で進めて「今」の時刻を見積もる（[`State::now`]）。
#[derive(Debug, Clone, Copy)]
struct Clock {
    time: Micros,
    known_at: Instant,
}

impl Clock {
    fn new(time: Micros) -> Clock {
        Clock {
            time,
            known_at: Instant::now(),
        }
    }

    /// サーバーの時計で進めた、今のコースの時刻。
    fn running(&self) -> Micros {
        self.time + self.known_at.elapsed().as_micros() as Micros
    }
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

    /// いちばん新しいセッションの ID。計測ボックスの入力はここに入れる。
    pub fn latest_session(&self) -> Option<i64> {
        self.sessions.read().unwrap().keys().next_back().copied()
    }

    /// イベントを追記して結果を計算し直す。届き済みの番号のイベント（再送）は無視する。
    pub fn append(&self, id: i64, append: AppendEvents) -> Result<Appended> {
        let AppendEvents { source, mut events } = append;
        if source.trim().is_empty() {
            return Err(Error::BadRequest("source is empty".into()));
        }
        let session = self.get(id)?;
        let mut state = session.state.lock().unwrap();
        let clock = state.clock.map(|clock| clock.running());
        let after_passages = state.after_passages();
        let mut next_id = state.next_passage_id();
        let mut stamped = false;
        for numbered in &mut events {
            if numbered.at_now {
                // 操作してから送るまでにかかった分（送り直したときなど）を戻す
                let clock = clock.map(|clock| clock - numbered.age_ms as Micros * MILLISECOND);
                match &mut numbered.event {
                    // 通過はその時刻のまま（ほかの通過との前後は計算で並べ直す）
                    Event::Passage(passage) => {
                        passage.time = clock.or(after_passages).unwrap_or(0);
                    }
                    // 列の操作などは、すでに届いた通過より後にする（すでに始まった走行の車を変えないように）
                    event => {
                        let at = at_mut(event).ok_or_else(|| {
                            Error::BadRequest("at_now is for events with a time".into())
                        })?;
                        // Option の比較では None < Some なので、片方しかなければそちらになる
                        *at = clock.max(after_passages).unwrap_or(0);
                    }
                }
                stamped = true;
            }
            if numbered.new_id {
                let Event::Passage(passage) = &mut numbered.event else {
                    return Err(Error::BadRequest("new_id is for passages".into()));
                };
                passage.id = next_id;
                next_id = PassageId(next_id.0 + 1);
            }
        }
        if stamped && state.clock.is_none() {
            // 計測ボックスの時刻が届いていなくても、ここからサーバーの時計でコースの時計を進める
            state.clock = Some(Clock::new(after_passages.unwrap_or(0)));
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
        state.log.extend(accepted);
        if count > 0 || stamped {
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
        if state.clock.is_some_and(|clock| time <= clock.time) {
            return Ok(());
        }
        state.clock = Some(Clock::new(time));
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
    /// 届いているどの通過記録よりも後の時刻（最も遅い通過の 1 µs 後）。
    ///
    /// 同じ時刻だと列の操作が通過より先に効くので、列の操作をこれより前にすると、すでに始まった走行の車が変わってしまう。
    fn after_passages(&self) -> Option<Micros> {
        self.log
            .iter()
            .filter_map(|event| match event {
                Event::Passage(passage) => Some(passage.time + 1),
                _ => None,
            })
            .max()
    }

    /// まだ使われていない通過記録の ID（今ある最大の ID の次）。
    fn next_passage_id(&self) -> PassageId {
        let max = self
            .log
            .iter()
            .filter_map(|event| match event {
                Event::Passage(passage) => Some(passage.id.0),
                _ => None,
            })
            .max();
        PassageId(max.map_or(1, |id| id + 1))
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
        outcome: compute(&info.config, &state.log, state.clock.map(|c| c.time)),
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
                    new_id: false,
                    age_ms: 0,
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

    #[test]
    fn manual_passages_get_new_ids_and_the_running_clock() {
        let hub = Hub::open(Store::open_in_memory().unwrap()).unwrap();
        let id = hub.create_session(new_session()).unwrap().id;
        hub.append(id, numbered(0, vec![pass(5, S, 10 * SECOND)]))
            .unwrap();
        hub.set_clock(id, 10 * SECOND).unwrap();

        // 手動のゴール。時刻と ID はサーバーが付ける
        let mut manual = numbered(
            1,
            vec![Event::Passage(Passage {
                id: PassageId(0),
                gate: G,
                time: 0,
                source: PassageSource::Manual,
            })],
        );
        manual.events[0].at_now = true;
        manual.events[0].new_id = true;
        std::thread::sleep(std::time::Duration::from_millis(20));
        hub.append(id, manual).unwrap();

        let session = hub.get(id).unwrap();
        let state = session.state.lock().unwrap();
        let Event::Passage(goal) = state.log[1] else {
            panic!("not a passage: {:?}", state.log[1]);
        };
        assert_eq!(goal.id, PassageId(6));
        // 時計の通知がなくても、サーバーの時計で進んでいる
        assert!(goal.time >= 10 * SECOND + 20 * MILLISECOND, "{}", goal.time);
        drop(state);
        let run = &hub.snapshot(id).unwrap().outcome.runs[0];
        assert_eq!(run.status, RunStatus::Finished);

        // 5 秒前に押した（送り直しで遅れた）手動の通過は、5 秒前の時刻になる
        let mut late = numbered(
            3,
            vec![Event::Passage(Passage {
                id: PassageId(0),
                gate: S,
                time: 0,
                source: PassageSource::Manual,
            })],
        );
        late.events[0].at_now = true;
        late.events[0].new_id = true;
        late.events[0].age_ms = 5_000;
        hub.append(id, late).unwrap();
        let session = hub.get(id).unwrap();
        let state = session.state.lock().unwrap();
        let Event::Passage(start) = state.log[2] else {
            panic!("not a passage: {:?}", state.log[2]);
        };
        assert_eq!(start.id, PassageId(7));
        assert!(start.time < 6 * SECOND, "{}", start.time);
        drop(state);

        // 通過記録でないイベントには使えない
        let mut bad = numbered(
            2,
            vec![Event::CancelReport {
                run: RunId(PassageId(5)),
            }],
        );
        bad.events[0].new_id = true;
        assert!(matches!(hub.append(id, bad), Err(Error::BadRequest(_))));
    }
}
