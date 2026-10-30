//! ログから割り当てを計算する。
//!
//! 毎回ログ全体から計算し直す。手順:
//!
//! 1. 通過記録を集め、遅延を補正し、無効にされたものと二重反応を除く。
//! 2. 走行への訂正（車の指定、コースアウトの報告、ペナルティ）を、走行の ID ごとにまとめる。
//! 3. 残った通過記録と出走待ちの列の操作を時刻順に処理し、通過を走行に割り当てる。
//!    割り当てと矛盾する通過が来たら、直前の割り当てを付け替える。
//! 4. 区間タイムなどを確かめて、確認を促す目印を付ける。

use std::collections::BTreeMap;

use crate::config::Config;
use crate::event::{CarChoice, Event, PassageSource, Penalty};
use crate::outcome::{
    CarSource, Disposition, OffCause, Outcome, PassageReport, Run, RunFlag, RunResult, RunStatus,
    Slot, Warning,
};
use crate::types::{CarId, GateId, Micros, PassageId, RunId};

/// ログから割り当てを計算する。
///
/// `now` を渡すと、その時刻までに最大走行時間を超えた走行や、報告済みの走行を外した状態にする。
pub fn compute(config: &Config, log: &[Event], now: Option<Micros>) -> Outcome {
    let mut warnings = Vec::new();
    let input = Input::collect(config, log, &mut warnings);
    let dispositions = debounce(config, &input);
    let corrections = Corrections::resolve(&input, &dispositions, &mut warnings);

    let mut state = State::new(config, &corrections);
    for item in timeline(&input, &dispositions) {
        state.settle(item.time);
        match item.kind {
            ItemKind::Enqueue(car) => state.queue.push(car),
            ItemKind::Dequeue(car) => match state.queue.iter().position(|&c| c == car) {
                Some(i) => {
                    state.queue.remove(i);
                }
                None => warnings.push(Warning::NotQueued(car)),
            },
            ItemKind::Passage(k) => {
                let p = &input.passages[k];
                state.pass(p.id, p.gate, p.time, &mut warnings);
            }
        }
    }
    if let Some(now) = now {
        state.settle(now);
    }

    corrections.warn_unused(&state.runs, &mut warnings);
    finish(config, input, dispositions, state, warnings)
}

// ---------------------------------------------------------------------------
// 1. 入力を集める

struct PassageIn {
    id: PassageId,
    gate: GateId,
    raw: Micros,
    time: Micros,
    source: PassageSource,
}

#[derive(Clone, Copy)]
struct Report {
    last_position: usize,
    at: Micros,
}

/// 訂正。ログでの位置（後のものが優先）と値。
type Latest<T> = BTreeMap<PassageId, (usize, T)>;

struct Input {
    passages: Vec<PassageIn>,
    index: BTreeMap<PassageId, usize>,
    void: BTreeMap<PassageId, bool>,
    cars: Latest<CarChoice>,
    reports: Latest<Option<Report>>,
    penalties: Latest<Penalty>,
    /// (時刻, ログでの位置, 操作)
    queue_ops: Vec<(Micros, usize, ItemKind)>,
}

impl Input {
    fn collect(config: &Config, log: &[Event], warnings: &mut Vec<Warning>) -> Input {
        let mut input = Input {
            passages: Vec::new(),
            index: BTreeMap::new(),
            void: BTreeMap::new(),
            cars: BTreeMap::new(),
            reports: BTreeMap::new(),
            penalties: BTreeMap::new(),
            queue_ops: Vec::new(),
        };
        for (i, event) in log.iter().enumerate() {
            match *event {
                Event::Passage(p) => {
                    if input.index.contains_key(&p.id) {
                        warnings.push(Warning::DuplicatePassage(p.id));
                        continue;
                    }
                    input.index.insert(p.id, input.passages.len());
                    input.passages.push(PassageIn {
                        id: p.id,
                        gate: p.gate,
                        raw: p.time,
                        time: p.time - config.gate(p.gate).delay,
                        source: p.source,
                    });
                }
                Event::SetVoid { passage, void } => {
                    input.void.insert(passage, void);
                }
                Event::Enqueue { car, at } => input.queue_ops.push((at, i, ItemKind::Enqueue(car))),
                Event::Dequeue { car, at } => input.queue_ops.push((at, i, ItemKind::Dequeue(car))),
                Event::SetCar { run, car } => {
                    input.cars.insert(run.0, (i, car));
                }
                Event::ReportOff {
                    run,
                    last_position,
                    at,
                } => {
                    let report = Report { last_position, at };
                    input.reports.insert(run.0, (i, Some(report)));
                }
                Event::CancelReport { run } => {
                    input.reports.insert(run.0, (i, None));
                }
                Event::SetPenalty { run, penalty } => {
                    input.penalties.insert(run.0, (i, penalty));
                }
            }
        }
        for &id in input.void.keys() {
            if !input.index.contains_key(&id) {
                warnings.push(Warning::UnknownPassage(id));
            }
        }
        input
    }
}

/// 無効にされた通過と二重反応を除く。残ったものは仮に `Orphan` とする。
fn debounce(config: &Config, input: &Input) -> Vec<Disposition> {
    let mut order: Vec<usize> = (0..input.passages.len()).collect();
    order.sort_by_key(|&k| (input.passages[k].time, input.passages[k].id));

    let mut dispositions = vec![Disposition::Orphan; input.passages.len()];
    let mut last_accepted: BTreeMap<GateId, (Micros, PassageId)> = BTreeMap::new();
    for k in order {
        let p = &input.passages[k];
        if input.void.get(&p.id) == Some(&true) {
            dispositions[k] = Disposition::Void;
            continue;
        }
        if let Some(&(t, by)) = last_accepted.get(&p.gate)
            && p.time - t < config.gate(p.gate).lockout
        {
            dispositions[k] = Disposition::Debounced { by };
            continue;
        }
        last_accepted.insert(p.gate, (p.time, p.id));
    }
    dispositions
}

// ---------------------------------------------------------------------------
// 2. 訂正を走行ごとにまとめる

/// 走行（スタートの通過記録の ID）ごとの訂正。
struct Corrections {
    cars: BTreeMap<PassageId, CarChoice>,
    reports: BTreeMap<PassageId, Report>,
    penalties: BTreeMap<PassageId, Penalty>,
}

impl Corrections {
    fn resolve(input: &Input, dispositions: &[Disposition], warnings: &mut Vec<Warning>) -> Self {
        // 二重反応として無視された通過を指す訂正は、それを無視させた通過（=実際の走行の ID）に付け替える。
        let resolve = |id: PassageId| -> Option<PassageId> {
            let &k = input.index.get(&id)?;
            match dispositions[k] {
                Disposition::Debounced { by } => Some(by),
                Disposition::Void => None,
                _ => Some(id),
            }
        };
        fn merge<T: Copy>(
            src: &Latest<T>,
            resolve: &dyn Fn(PassageId) -> Option<PassageId>,
            warnings: &mut Vec<Warning>,
        ) -> BTreeMap<PassageId, T> {
            let mut merged: Latest<T> = BTreeMap::new();
            for (&id, &(i, value)) in src {
                let Some(target) = resolve(id) else {
                    warnings.push(Warning::NoSuchRun(RunId(id)));
                    continue;
                };
                if merged.get(&target).is_none_or(|&(j, _)| i > j) {
                    merged.insert(target, (i, value));
                }
            }
            merged.into_iter().map(|(id, (_, v))| (id, v)).collect()
        }

        let cars = merge(&input.cars, &resolve, warnings);
        let reports = merge(&input.reports, &resolve, warnings)
            .into_iter()
            .filter_map(|(id, r)| Some((id, r?)))
            .collect();
        let penalties = merge(&input.penalties, &resolve, warnings);
        Corrections {
            cars,
            reports,
            penalties,
        }
    }

    /// 走行にならなかった通過を指す訂正を警告する。
    fn warn_unused(&self, runs: &[RunState], warnings: &mut Vec<Warning>) {
        let is_run = |id: &PassageId| runs.iter().any(|r| r.id.0 == *id);
        let mut ids: Vec<PassageId> = self
            .cars
            .keys()
            .chain(self.reports.keys())
            .chain(self.penalties.keys())
            .copied()
            .filter(|id| !is_run(id))
            .collect();
        ids.sort();
        ids.dedup();
        warnings.extend(ids.into_iter().map(|id| Warning::NoSuchRun(RunId(id))));
    }
}

// ---------------------------------------------------------------------------
// 3. 時刻順に処理する

#[derive(Clone, Copy)]
enum ItemKind {
    Enqueue(CarId),
    Dequeue(CarId),
    /// `Input::passages` の添字。
    Passage(usize),
}

struct Item {
    time: Micros,
    kind: ItemKind,
}

/// 出走待ちの列の操作と、残った通過記録を時刻順に並べる。同じ時刻なら列の操作が先。
fn timeline(input: &Input, dispositions: &[Disposition]) -> Vec<Item> {
    let mut keyed: Vec<((Micros, u8, u64), Item)> = Vec::new();
    for &(time, i, kind) in &input.queue_ops {
        keyed.push(((time, 0, i as u64), Item { time, kind }));
    }
    for (k, p) in input.passages.iter().enumerate() {
        if dispositions[k] == Disposition::Orphan {
            let item = Item {
                time: p.time,
                kind: ItemKind::Passage(k),
            };
            keyed.push(((p.time, 1, p.id.0), item));
        }
    }
    keyed.sort_by_key(|(key, _)| *key);
    keyed.into_iter().map(|(_, item)| item).collect()
}

struct RunState {
    id: RunId,
    car: Option<CarId>,
    car_source: CarSource,
    start_time: Micros,
    passes: Vec<Slot>,
    /// 最後に通過したポジションとその時刻。次に通過するのは `last_position + 1` 以降。
    last_position: usize,
    last_time: Micros,
    status: RunStatus,
    report: Option<Report>,
    flags: Vec<RunFlag>,
}

impl RunState {
    /// この走行が、`gate` の時刻 `time` の通過を受け取れるならそのポジション。
    ///
    /// 中間のポジションは飛ばしてよい（反応の抜け）。ただし最小区間タイムの合計を満たすこと。
    fn accepts(&self, config: &Config, gate: GateId, time: Micros) -> Option<usize> {
        let course = &config.course;
        let limit = match self.report {
            Some(r) => r.last_position.min(course.finish()),
            None => course.finish(),
        };
        for q in self.last_position + 1..=limit {
            if time - self.last_time < config.min_between(self.last_position, q) {
                return None;
            }
            if course.gate_at(q) == gate {
                return Some(q);
            }
        }
        None
    }

    fn take_off(&mut self, cause: OffCause, at: Micros) {
        self.status = RunStatus::Off { cause, at };
    }
}

struct State<'a> {
    config: &'a Config,
    corrections: &'a Corrections,
    runs: Vec<RunState>,
    queue: Vec<CarId>,
    /// 区間ごとの、これまでに観測した区間タイム（押し出す車を選ぶ基準）。
    observed: Vec<Vec<Micros>>,
}

impl<'a> State<'a> {
    fn new(config: &'a Config, corrections: &'a Corrections) -> Self {
        State {
            config,
            corrections,
            runs: Vec::new(),
            queue: Vec::new(),
            observed: vec![Vec::new(); config.course.segments()],
        }
    }

    /// 時刻 `t` より前に、最大走行時間か最大区間タイムを超えた走行と、報告された走行を外す。
    fn settle(&mut self, t: Micros) {
        let config = self.config;
        for run in &mut self.runs {
            if run.status != RunStatus::Running {
                continue;
            }
            let timeout = config
                .max_run_time
                .map(|m| (run.start_time + m, OffCause::TimedOut));
            let segment_timeout = config
                .max_segment(run.last_position)
                .map(|m| (run.last_time + m, OffCause::TimedOut));
            let reported = run.report.map(|r| (r.at, OffCause::Reported));
            let first = [timeout, segment_timeout, reported]
                .into_iter()
                .flatten()
                .filter(|&(at, _)| at < t)
                .min_by_key(|&(at, _)| at);
            if let Some((at, cause)) = first {
                run.take_off(cause, at);
            }
        }
    }

    fn running(&self) -> impl Iterator<Item = (usize, &RunState)> {
        self.runs
            .iter()
            .enumerate()
            .filter(|(_, r)| r.status == RunStatus::Running)
    }

    /// 通過を割り当てる。
    ///
    /// 走行中の車のうち、このゲートを次に（中間を飛ばす場合は最も少なく飛ばして）通るはずの、最も古い走行に割り当てる。
    /// どの走行にも当てはまらず、スタートのゲートなら新しい走行を始める。どちらでもなければ、どこにも割り当てない。
    fn pass(&mut self, id: PassageId, gate: GateId, time: Micros, warnings: &mut Vec<Warning>) {
        let best = self
            .running()
            .filter_map(|(i, r)| {
                let q = r.accepts(self.config, gate, time)?;
                Some((q - r.last_position - 1, i, q))
            })
            .min();
        if self.repair(id, gate, time, best.map(|(_, i, _)| i)) {
            return;
        }
        if let Some((_, i, position)) = best {
            self.advance(i, id, time, position);
        } else if gate == self.config.course.start_gate() {
            self.start(id, time, warnings);
        }
    }

    /// この通過との矛盾から、直前の割り当ての誤りを見つけて直す。直したら `true`。
    ///
    /// 走行 A が次に通るはずのゲートの通過なのに、A の最後の通過（中間）から最小区間タイムが経っていない。
    /// これは、その中間の通過が A ではなく後続の走行 B のもので、A はそこで反応が抜けていたと考えると説明できる。
    /// B がその中間の通過を受け取れて、A がその中間を飛ばしてこの通過を受け取れるなら、付け替える。
    ///
    /// `best` は通常の割り当て先。A より古い走行が受け取れるなら、そちらを優先して何もしない。
    fn repair(&mut self, id: PassageId, gate: GateId, time: Micros, best: Option<usize>) -> bool {
        let config = self.config;
        let course = &config.course;
        let Some((a, run_a)) = self.running().find(|(_, r)| {
            let q = r.last_position + 1;
            q < course.len()
                && course.gate_at(q) == gate
                && r.report.is_none_or(|rep| q <= rep.last_position)
        }) else {
            return false;
        };
        if best.is_some_and(|b| b <= a) {
            return false;
        }
        let (p, q) = (run_a.last_position, run_a.last_position + 1);
        if p == 0 || time - run_a.last_time >= config.min_between(p, q) {
            return false;
        }
        let Slot::Passed {
            passage: x,
            time: tx,
        } = run_a.passes[p]
        else {
            return false;
        };
        // A のその前の通過から、中間を飛ばしてこの通過を受け取れるか
        let Some((pp, tp)) = run_a.passes[..p]
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, slot)| Some((i, slot.time()?)))
        else {
            return false;
        };
        if time - tp < config.min_between(pp, q) {
            return false;
        }
        // 中間の通過を受け取れる後続の走行
        let Some(b) = self
            .running()
            .filter(|&(i, _)| i > a)
            .find(|(_, r)| r.last_time < tx && r.accepts(config, course.gate_at(p), tx) == Some(p))
            .map(|(i, _)| i)
        else {
            return false;
        };

        let run_a = &mut self.runs[a];
        run_a.last_position = pp;
        run_a.last_time = tp;
        run_a.flags.push(RunFlag::Reassigned { position: p });
        self.advance(b, x, tx, p);
        self.runs[b].flags.push(RunFlag::Reassigned { position: p });
        self.advance(a, id, time, q);
        true
    }

    fn advance(&mut self, i: usize, id: PassageId, time: Micros, position: usize) {
        let finish = self.config.course.finish();
        let run = &mut self.runs[i];
        for slot in &mut run.passes[run.last_position + 1..position] {
            *slot = Slot::Missed;
        }
        if position == run.last_position + 1 {
            self.observed[run.last_position].push(time - run.last_time);
        }
        run.passes[position] = Slot::Passed { passage: id, time };
        run.last_position = position;
        run.last_time = time;
        if position == finish {
            run.status = RunStatus::Finished;
        } else if run.report.is_some_and(|r| r.last_position == position) {
            run.take_off(OffCause::Reported, time);
        }
    }

    /// 新しい走行を始める。車は `SetCar` の指定があればそれ（列にいれば列から外す）、なければ列の先頭。
    fn start(&mut self, id: PassageId, time: Micros, warnings: &mut Vec<Warning>) {
        let run_id = RunId(id);
        if let Some(max) = self.config.max_on_course {
            while self.running().count() >= max.max(1) {
                let victim = self.choose_push_out(time);
                self.runs[victim].take_off(OffCause::PushedOut { by: run_id }, time);
            }
        }

        let (car, car_source) = match self.corrections.cars.get(&id) {
            Some(&CarChoice::Car(car)) => {
                if let Some(i) = self.queue.iter().position(|&c| c == car) {
                    self.queue.remove(i);
                }
                (Some(car), CarSource::Manual)
            }
            Some(&CarChoice::NoCar) => (None, CarSource::Manual),
            Some(&CarChoice::Auto) | None if !self.queue.is_empty() => {
                (Some(self.queue.remove(0)), CarSource::Queue)
            }
            Some(&CarChoice::Auto) | None => (None, CarSource::Unassigned),
        };

        let mut report = self.corrections.reports.get(&id).copied();
        if report.is_some_and(|r| r.last_position >= self.config.course.finish()) {
            warnings.push(Warning::InvalidReport(run_id));
            report = None;
        }

        let mut passes = vec![Slot::NotReached; self.config.course.len()];
        passes[0] = Slot::Passed { passage: id, time };
        let mut run = RunState {
            id: run_id,
            car,
            car_source,
            start_time: time,
            passes,
            last_position: 0,
            last_time: time,
            status: RunStatus::Running,
            report,
            flags: Vec::new(),
        };
        if report.is_some_and(|r| r.last_position == 0) {
            run.take_off(OffCause::Reported, time);
        }
        self.runs.push(run);
    }

    /// 押し出す走行を選ぶ。「進み具合のわりに最も遅れている」走行。
    ///
    /// 最後の通過からの経過時間を、次の区間のこれまでの区間タイムの中央値で割った値が最も大きい走行。
    /// 中央値がまだない区間にいる走行があるときは、最も古い走行。
    fn choose_push_out(&self, time: Micros) -> usize {
        let mut oldest = None;
        let mut best: Option<(i64, usize)> = None;
        let mut comparable = true;
        for (i, run) in self.running() {
            oldest.get_or_insert(i);
            match median(&self.observed[run.last_position]) {
                Some(m) if m > 0 => {
                    let score = (time - run.last_time) * 1000 / m;
                    if best.is_none_or(|(s, _)| score > s) {
                        best = Some((score, i));
                    }
                }
                _ => comparable = false,
            }
        }
        match (comparable, best) {
            (true, Some((_, i))) => i,
            _ => oldest.expect("push out requires a running run"),
        }
    }
}

fn median(values: &[Micros]) -> Option<Micros> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    Some(sorted[sorted.len() / 2])
}

// ---------------------------------------------------------------------------
// 4. 結果をまとめる

fn finish(
    config: &Config,
    input: Input,
    mut dispositions: Vec<Disposition>,
    state: State<'_>,
    warnings: Vec<Warning>,
) -> Outcome {
    let State {
        corrections,
        runs,
        queue,
        ..
    } = state;

    let mut runs: Vec<Run> = runs
        .into_iter()
        .map(|r| {
            let penalty = corrections
                .penalties
                .get(&r.id.0)
                .copied()
                .unwrap_or_default();
            let mut run = Run {
                id: r.id,
                car: r.car,
                car_source: r.car_source,
                start_time: r.start_time,
                passes: r.passes,
                status: r.status,
                penalty,
                result: RunResult::Running,
                flags: r.flags,
            };
            run.result = result(config, &run);
            run
        })
        .collect();

    // 全走行の区間タイムの中央値
    let medians: Vec<Option<Micros>> = (0..config.course.segments())
        .map(|s| {
            let times: Vec<Micros> = runs.iter().filter_map(|r| r.segment(s)).collect();
            if times.len() >= 3 {
                median(&times)
            } else {
                None
            }
        })
        .collect();

    for run in &mut runs {
        for (position, slot) in run.passes.iter().enumerate() {
            if *slot == Slot::Missed {
                run.flags.push(RunFlag::MissingPass { position });
            }
        }
        for (segment, median) in medians.iter().enumerate() {
            let (Some(time), Some(median)) = (run.segment(segment), *median) else {
                continue;
            };
            let percent = time * 100 / median.max(1);
            if config
                .slow_segment_percent
                .is_some_and(|p| percent > p as i64)
            {
                run.flags.push(RunFlag::SlowSegment {
                    segment,
                    time,
                    median,
                });
            }
            if config
                .fast_segment_percent
                .is_some_and(|p| percent < p as i64)
            {
                run.flags.push(RunFlag::FastSegment {
                    segment,
                    time,
                    median,
                });
            }
        }
        if run.car.is_none() {
            run.flags.push(RunFlag::NoCar);
        }
    }

    for run in &runs {
        for (position, slot) in run.passes.iter().enumerate() {
            if let Some(passage) = slot.passage() {
                dispositions[input.index[&passage]] = Disposition::Assigned {
                    run: run.id,
                    position,
                };
            }
        }
    }

    let mut passages: Vec<PassageReport> = input
        .passages
        .iter()
        .zip(dispositions)
        .map(|(p, disposition)| PassageReport {
            id: p.id,
            gate: p.gate,
            raw_time: p.raw,
            time: p.time,
            source: p.source,
            disposition,
        })
        .collect();
    passages.sort_by_key(|p| (p.time, p.id));

    Outcome {
        runs,
        passages,
        queue,
        warnings,
    }
}

fn result(config: &Config, run: &Run) -> RunResult {
    match run.status {
        RunStatus::Running => RunResult::Running,
        RunStatus::Off { .. } => RunResult::Dnf,
        RunStatus::Finished if run.penalty.dnf => RunResult::Dnf,
        RunStatus::Finished if run.penalty.miss_course => RunResult::MissCourse,
        RunStatus::Finished => {
            let total = run.total().expect("finished run has a finish time");
            RunResult::Time(total + run.penalty.pylons as Micros * config.pylon_penalty)
        }
    }
}
