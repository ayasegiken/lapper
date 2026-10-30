//! シナリオから、実際に起きたこと（正解）と、計測ロジックに入るログを作る。

use lapper_core::{
    CarChoice, CarId, Config, Event, GateId, MILLISECOND, Micros, Passage, PassageId,
    PassageSource, RunId,
};

use crate::rng::Rng;
use crate::scenario::{Scenario, seconds};

/// 生成したセッション。
#[derive(Debug, Clone)]
pub struct Simulation {
    pub config: Config,
    /// 計測ロジックに届いた順のログ。
    pub log: Vec<Event>,
    /// 実際の走行。スタート順。
    pub truth: Vec<TruthRun>,
    /// セッションの終わりの時刻（最後のイベントから最大走行時間が過ぎた後）。
    pub end: Micros,
}

/// 実際の走行。
#[derive(Debug, Clone)]
pub struct TruthRun {
    pub car: CarId,
    /// ポジションごとの実際の通過時刻。通過しなかったポジションは `None`。
    pub times: Vec<Option<Micros>>,
    /// ポジションごとの通過記録の ID。光電管が反応しなかった場合は `None`。
    pub passages: Vec<Option<PassageId>>,
    /// コースアウトした場合、最後に通過したポジション。
    pub off_after: Option<usize>,
    /// コースアウトが報告されたか。
    pub reported: bool,
    /// 出走待ちの列に入らずにスタートしたか、列の順と入れ替わってスタートしたか。
    pub out_of_queue: bool,
    /// 運営が訂正したか（車の指定、またはスタートが記録されなかった車を列から外した）。
    pub fixed: bool,
}

impl TruthRun {
    pub fn finished(&self) -> bool {
        self.off_after.is_none()
    }

    /// 実際のタイム（完走した場合）。
    pub fn total(&self) -> Option<Micros> {
        if !self.finished() {
            return None;
        }
        Some((*self.times.last()?)? - self.times[0]?)
    }

    /// コース上からいなくなった時刻（ゴールした時刻か、コースアウトした時刻）。
    fn end(&self, left_at: Micros) -> Micros {
        if self.finished() {
            self.times.last().copied().flatten().unwrap_or(left_at)
        } else {
            left_at
        }
    }
}

struct Planned {
    car: CarId,
    /// 出走待ちの列での位置。
    nominal: usize,
    queued: bool,
    swapped: bool,
}

/// 届いた時刻つきのイベント。
struct Arrival {
    at: Micros,
    order: u8,
    event: Event,
}

/// 通過記録の候補（ID を付ける前）。
struct RawPassage {
    gate: GateId,
    time: Micros,
    /// 実際の走行の通過なら (走行, ポジション)。
    truth: Option<(usize, usize)>,
    /// 車と関係ない誤反応か。
    noise: bool,
}

pub fn generate(scenario: &Scenario, seed: u64) -> Simulation {
    let mut rng = Rng::new(seed);
    let faults = scenario.faults;
    let config = scenario.config();
    let positions = scenario.course.len();

    // 車ごとの速さ
    let pace: Vec<f64> = (0..scenario.cars)
        .map(|_| rng.normal(1.0, scenario.driver_spread).clamp(0.8, 1.3))
        .collect();

    // 出走待ちの列の順。運営は車を列の順に並べる。
    let queue_order: Vec<CarId> = (0..scenario.heats)
        .flat_map(|_| (1..=scenario.cars).map(CarId))
        .collect();
    // 実際にスタートする順。ときどき列にいない車や、入れ替わった車がいる。
    let mut plan: Vec<Planned> = queue_order
        .iter()
        .enumerate()
        .map(|(nominal, &car)| Planned {
            car,
            nominal,
            queued: !rng.chance(faults.unqueued),
            swapped: false,
        })
        .collect();
    let mut k = 0;
    while k + 1 < plan.len() {
        if rng.chance(faults.swap) {
            plan.swap(k, k + 1);
            plan[k].swapped = true;
            plan[k + 1].swapped = true;
            k += 2;
        } else {
            k += 1;
        }
    }

    // 走行を時刻に並べる
    let mut truth: Vec<TruthRun> = Vec::new();
    let mut ends: Vec<Micros> = Vec::new();
    let mut earliest = seconds(60.0);
    for planned in &plan {
        let car = planned.car;
        let durations: Vec<Micros> = scenario
            .segment_times
            .iter()
            .map(|&t| {
                let factor = pace[car.0 as usize - 1] * rng.normal(1.0, scenario.run_jitter);
                seconds(t * factor.max(0.7))
            })
            .collect();
        let off_after = rng
            .chance(faults.course_out)
            .then(|| rng.below(positions - 1));
        let off_fraction = rng.uniform(0.2, 0.8);
        // スターターがコースアウトに気付くまでの時間
        let notice = seconds(rng.uniform(5.0, 20.0));

        let mut start = earliest;
        let (times, left_at) = loop {
            let times = schedule(start, &durations, off_after);
            let left_at = match off_after {
                Some(l) => times[l].unwrap() + (durations[l] as f64 * off_fraction) as Micros,
                None => times[positions - 1].unwrap(),
            };
            // 同時走行の台数を守る
            let on_course: Vec<Micros> = ends.iter().copied().filter(|&e| e > start).collect();
            if on_course.len() >= scenario.max_on_course {
                let mut sorted = on_course;
                sorted.sort();
                start = sorted[sorted.len() - scenario.max_on_course] + seconds(2.0);
                continue;
            }
            // 前の車を追い越さない、同じゲートの通過が近すぎない
            if let Some(shift) = conflict(scenario, &truth, &times) {
                start += shift;
                continue;
            }
            break (times, left_at);
        };
        let run = TruthRun {
            car,
            times,
            passages: vec![None; positions],
            off_after,
            reported: false,
            out_of_queue: !planned.queued || planned.swapped,
            fixed: false,
        };
        let end = run.end(left_at);
        ends.push(if run.finished() { end } else { end + notice });
        earliest = start + seconds(scenario.start_interval);
        truth.push(run);
    }

    // 通過記録
    let mut raw: Vec<RawPassage> = Vec::new();
    for (r, run) in truth.iter().enumerate() {
        for (position, time) in run.times.iter().enumerate() {
            let Some(time) = *time else { continue };
            if rng.chance(faults.miss) {
                continue;
            }
            let gate = scenario.course[position];
            let time = time + scenario.delay(gate);
            raw.push(RawPassage {
                gate,
                time,
                truth: Some((r, position)),
                noise: false,
            });
            if rng.chance(faults.chatter) {
                let bounce = (rng.uniform(20.0, 300.0) * MILLISECOND as f64) as Micros;
                raw.push(RawPassage {
                    gate,
                    time: time + bounce,
                    truth: None,
                    noise: false,
                });
            }
        }
    }
    let session_end = ends.iter().copied().max().unwrap_or(0) + seconds(60.0);
    let hours = session_end as f64 / seconds(3600.0) as f64;
    let mut gates = scenario.course.clone();
    gates.sort();
    gates.dedup();
    for _ in 0..(faults.noise_per_hour * hours).round() as usize {
        raw.push(RawPassage {
            gate: gates[rng.below(gates.len())],
            time: seconds(rng.uniform(0.0, session_end as f64 / 1e6)),
            truth: None,
            noise: true,
        });
    }

    // 計測ボックスの連番の順に ID を付ける
    raw.sort_by_key(|p| p.time);
    let mut arrivals: Vec<Arrival> = Vec::new();
    for (i, p) in raw.iter().enumerate() {
        let id = PassageId(i as u64 + 1);
        match p.truth {
            Some((r, position)) => truth[r].passages[position] = Some(id),
            // スタートでの誤反応は、スターターが気付いて無効にする
            None if p.noise && p.gate == scenario.course[0] && rng.chance(faults.fix) => {
                arrivals.push(Arrival {
                    at: p.time + seconds(rng.uniform(10.0, 120.0)),
                    order: 2,
                    event: Event::SetVoid {
                        passage: id,
                        void: true,
                    },
                });
            }
            None => {}
        }
        arrivals.push(Arrival {
            at: p.time,
            order: 1,
            event: Event::Passage(Passage {
                id,
                gate: p.gate,
                time: p.time,
                source: PassageSource::Sensor,
            }),
        });
    }

    // 出走待ちの列。列の k 番目の車は、2 台前のスタートの 30 秒前に列に入る。
    let mut queued = vec![true; queue_order.len()];
    for planned in &plan {
        queued[planned.nominal] = planned.queued;
    }
    for (k, &car) in queue_order.iter().enumerate() {
        if !queued[k] {
            continue;
        }
        let at = truth[k.saturating_sub(2)].times[0].unwrap() - seconds(30.0);
        arrivals.push(Arrival {
            at,
            order: 0,
            event: Event::Enqueue { car, at },
        });
    }

    // 運営と参加者からの訂正
    for (k, run) in truth.iter_mut().enumerate() {
        let Some(start_id) = run.passages[0] else {
            // スタートが記録されなかった。この車を、スタートの時点で列から外したことにする。
            let start = run.times[0].unwrap();
            if plan[k].queued && rng.chance(faults.fix) {
                run.fixed = true;
                arrivals.push(Arrival {
                    at: start + seconds(rng.uniform(10.0, 120.0)),
                    order: 2,
                    event: Event::Dequeue {
                        car: run.car,
                        at: start,
                    },
                });
            }
            continue;
        };
        let start = run.times[0].unwrap();
        let id = RunId(start_id);
        if run.out_of_queue && rng.chance(faults.fix) {
            run.fixed = true;
            let at = start + seconds(rng.uniform(10.0, 120.0));
            arrivals.push(Arrival {
                at,
                order: 2,
                event: Event::SetCar {
                    run: id,
                    car: CarChoice::Car(run.car),
                },
            });
        }
        if let Some(last_position) = run.off_after
            && rng.chance(faults.report)
        {
            run.reported = true;
            let at = run.times[last_position].unwrap() + seconds(rng.uniform(10.0, 60.0));
            arrivals.push(Arrival {
                at,
                order: 2,
                event: Event::ReportOff {
                    run: id,
                    last_position,
                    at,
                },
            });
        }
    }

    arrivals.sort_by_key(|a| (a.at, a.order));
    // 走行中のまま残る走行がないように、最大走行時間が過ぎるまで待つ
    let end =
        arrivals.last().map_or(0, |a| a.at).max(session_end) + config.max_run_time.unwrap_or(0);
    Simulation {
        config,
        log: arrivals.into_iter().map(|a| a.event).collect(),
        truth,
        end,
    }
}

/// スタートの時刻と区間タイムから、ポジションごとの通過時刻を出す。
fn schedule(start: Micros, durations: &[Micros], off_after: Option<usize>) -> Vec<Option<Micros>> {
    let last = off_after.unwrap_or(durations.len());
    let mut times = vec![None; durations.len() + 1];
    let mut t = start;
    times[0] = Some(t);
    for (s, d) in durations.iter().enumerate().take(last) {
        t += d;
        times[s + 1] = Some(t);
    }
    times
}

/// 前の走行との衝突があれば、スタートを遅らせる時間。
fn conflict(scenario: &Scenario, earlier: &[TruthRun], times: &[Option<Micros>]) -> Option<Micros> {
    let gap = seconds(scenario.min_gap);
    let mut shift = 0;
    for run in earlier {
        for (q, t) in times.iter().enumerate() {
            let Some(t) = *t else { continue };
            // 同じポジションでは前の車より gap 以上後
            if let Some(prev) = run.times[q]
                && t < prev + gap
            {
                shift = shift.max(prev + gap - t);
            }
            // 同じゲートの通過どうしは gap 以上離す
            for (p, prev) in run.times.iter().enumerate() {
                let Some(prev) = *prev else { continue };
                if p != q && scenario.course[p] == scenario.course[q] && (t - prev).abs() < gap {
                    shift = shift.max(prev + gap - t);
                }
            }
        }
    }
    (shift > 0).then_some(shift)
}
