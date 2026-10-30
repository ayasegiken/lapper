use lapper_core::*;

const S: GateId = GateId(0);
const I1: GateId = GateId(1);
const I2: GateId = GateId(2);
const G: GateId = GateId(3);

fn s(seconds: f64) -> Micros {
    (seconds * SECOND as f64).round() as Micros
}

/// ログを組み立てる補助。通過記録の ID は追加順の連番。
#[derive(Default)]
struct Log {
    events: Vec<Event>,
    next_id: u64,
}

impl Log {
    fn pass(&mut self, gate: GateId, seconds: f64) -> PassageId {
        self.next_id += 1;
        let id = PassageId(self.next_id);
        self.events.push(Event::Passage(Passage {
            id,
            gate,
            time: s(seconds),
            source: PassageSource::Sensor,
        }));
        id
    }

    fn queue(&mut self, car: u32, seconds: f64) {
        self.events.push(Event::Enqueue {
            car: CarId(car),
            at: s(seconds),
        });
    }

    fn push(&mut self, event: Event) {
        self.events.push(event);
    }
}

fn gymkhana() -> Config {
    let mut config = Config::new(Course::new(vec![S, I1, I2, G]).unwrap());
    config.min_segment = vec![s(5.0), s(5.0), s(5.0)];
    config
}

fn run_of(outcome: &Outcome, start: PassageId) -> &Run {
    outcome.run(RunId(start)).expect("run exists")
}

#[test]
fn single_run_with_splits() {
    let mut log = Log::default();
    log.queue(7, 0.0);
    let start = log.pass(S, 10.0);
    log.pass(I1, 30.5);
    log.pass(I2, 50.25);
    log.pass(G, 70.125);

    let outcome = compute(&gymkhana(), &log.events, None);
    let run = run_of(&outcome, start);
    assert_eq!(run.car, Some(CarId(7)));
    assert_eq!(run.car_source, CarSource::Queue);
    assert_eq!(run.status, RunStatus::Finished);
    assert_eq!(run.split(1), Some(s(20.5)));
    assert_eq!(run.split(2), Some(s(40.25)));
    assert_eq!(run.segment(2), Some(s(19.875)));
    assert_eq!(run.result, RunResult::Time(s(60.125)));
    assert!(run.flags.is_empty());
    assert!(outcome.queue.is_empty());
}

#[test]
fn concurrent_runs_are_assigned_in_order() {
    let mut log = Log::default();
    for car in 1..=3 {
        log.queue(car, 0.0);
    }
    let a = log.pass(S, 10.0);
    log.pass(I1, 30.0);
    let b = log.pass(S, 30.5);
    log.pass(I2, 50.0);
    log.pass(I1, 51.0);
    let c = log.pass(S, 52.0);
    log.pass(G, 70.0);
    log.pass(I2, 71.0);
    log.pass(I1, 72.5);
    log.pass(G, 92.0);
    log.pass(I2, 93.5);
    log.pass(G, 114.0);

    let outcome = compute(&gymkhana(), &log.events, None);
    assert_eq!(outcome.runs.len(), 3);
    let cars: Vec<_> = outcome.runs.iter().map(|r| r.car).collect();
    assert_eq!(cars, vec![Some(CarId(1)), Some(CarId(2)), Some(CarId(3))]);
    assert_eq!(run_of(&outcome, a).total(), Some(s(60.0)));
    assert_eq!(run_of(&outcome, b).total(), Some(s(61.5)));
    assert_eq!(run_of(&outcome, c).total(), Some(s(62.0)));
    assert_eq!(outcome.orphans().count(), 0);
}

#[test]
fn start_without_queued_car_is_unassigned_until_set() {
    let mut log = Log::default();
    let start = log.pass(S, 10.0);
    log.pass(I1, 30.0);
    log.pass(I2, 50.0);
    log.pass(G, 70.0);

    let outcome = compute(&gymkhana(), &log.events, None);
    let run = run_of(&outcome, start);
    assert_eq!(run.car, None);
    assert_eq!(run.car_source, CarSource::Unassigned);
    assert!(run.flags.contains(&RunFlag::NoCar));

    log.push(Event::SetCar {
        run: RunId(start),
        car: CarChoice::Car(CarId(5)),
    });
    let outcome = compute(&gymkhana(), &log.events, None);
    let run = run_of(&outcome, start);
    assert_eq!(run.car, Some(CarId(5)));
    assert_eq!(run.car_source, CarSource::Manual);
    assert!(run.flags.is_empty());
}

#[test]
fn correcting_a_swapped_start_fixes_the_following_runs() {
    // 列は 1, 2, 3 の順だが、実際には 2 が先にスタートした。
    let mut log = Log::default();
    for car in 1..=3 {
        log.queue(car, 0.0);
    }
    let first = log.pass(S, 10.0);
    let second = log.pass(S, 40.0);
    let third = log.pass(S, 70.0);

    let outcome = compute(&gymkhana(), &log.events, None);
    let cars: Vec<_> = outcome.runs.iter().map(|r| r.car).collect();
    assert_eq!(cars, vec![Some(CarId(1)), Some(CarId(2)), Some(CarId(3))]);

    // 最初の走行を 2 に訂正するだけで、2 番目は 1 に、3 番目は 3 のままになる。
    log.push(Event::SetCar {
        run: RunId(first),
        car: CarChoice::Car(CarId(2)),
    });
    let outcome = compute(&gymkhana(), &log.events, None);
    assert_eq!(run_of(&outcome, first).car, Some(CarId(2)));
    assert_eq!(run_of(&outcome, second).car, Some(CarId(1)));
    assert_eq!(run_of(&outcome, third).car, Some(CarId(3)));

    // 2 番目を 1 と指定し直しても同じ結果になる。
    log.push(Event::SetCar {
        run: RunId(second),
        car: CarChoice::Car(CarId(1)),
    });
    let outcome = compute(&gymkhana(), &log.events, None);
    assert_eq!(run_of(&outcome, second).car, Some(CarId(1)));
    assert_eq!(run_of(&outcome, third).car, Some(CarId(3)));
    assert!(outcome.queue.is_empty());
}

#[test]
fn correcting_only_the_later_run_of_a_swap_needs_both_corrections() {
    // 列は 1, 2 の順だが、2 が先にスタートした。後の走行だけを 1 と訂正しても、先の走行は列の先頭の 1 のまま。
    let mut log = Log::default();
    log.queue(1, 0.0);
    log.queue(2, 0.0);
    let first = log.pass(S, 10.0);
    let second = log.pass(S, 40.0);
    log.push(Event::SetCar {
        run: RunId(second),
        car: CarChoice::Car(CarId(1)),
    });
    let outcome = compute(&gymkhana(), &log.events, None);
    assert_eq!(run_of(&outcome, first).car, Some(CarId(1)));
    assert_eq!(outcome.queue, vec![CarId(2)]);

    log.push(Event::SetCar {
        run: RunId(first),
        car: CarChoice::Car(CarId(2)),
    });
    let outcome = compute(&gymkhana(), &log.events, None);
    assert_eq!(run_of(&outcome, first).car, Some(CarId(2)));
    assert_eq!(run_of(&outcome, second).car, Some(CarId(1)));
    assert!(outcome.queue.is_empty());
}

#[test]
fn manual_car_of_a_later_heat_does_not_disturb_earlier_runs() {
    // 2 本目は列に入らずに走り、訂正された。1 本目の列の割り当てには影響しない。
    let mut log = Log::default();
    log.queue(1, 0.0);
    log.queue(2, 0.0);
    let first = log.pass(S, 10.0);
    let second = log.pass(S, 40.0);
    let again = log.pass(S, 100.0);
    log.push(Event::SetCar {
        run: RunId(again),
        car: CarChoice::Car(CarId(1)),
    });
    let outcome = compute(&gymkhana(), &log.events, None);
    assert_eq!(run_of(&outcome, first).car, Some(CarId(1)));
    assert_eq!(run_of(&outcome, second).car, Some(CarId(2)));
    assert_eq!(run_of(&outcome, again).car, Some(CarId(1)));
}

#[test]
fn shared_start_finish_gate_uses_min_segment_time() {
    let sg = GateId(0);
    let mut config = Config::new(Course::new(vec![sg, sg]).unwrap());
    config.min_segment = vec![s(40.0)];
    let mut log = Log::default();
    log.queue(1, 0.0);
    log.queue(2, 0.0);
    let a = log.pass(sg, 10.0);
    let b = log.pass(sg, 40.0); // a の 30 秒後: 最小 40 秒に満たないので新しい走行
    log.pass(sg, 70.0); // a のゴール
    log.pass(sg, 101.0); // b のゴール

    let outcome = compute(&config, &log.events, None);
    assert_eq!(outcome.runs.len(), 2);
    assert_eq!(run_of(&outcome, a).total(), Some(s(60.0)));
    assert_eq!(run_of(&outcome, b).total(), Some(s(61.0)));
    assert_eq!(run_of(&outcome, b).car, Some(CarId(2)));
}

#[test]
fn course_with_repeated_gate() {
    let (a, b) = (GateId(1), GateId(2));
    let config = Config::new(Course::new(vec![S, a, b, a, G]).unwrap());
    let mut log = Log::default();
    let start = log.pass(S, 0.0);
    let first_a = log.pass(a, 10.0);
    log.pass(b, 20.0);
    let second_a = log.pass(a, 30.0);
    log.pass(G, 40.0);

    let outcome = compute(&config, &log.events, None);
    let run = run_of(&outcome, start);
    assert_eq!(run.status, RunStatus::Finished);
    assert_eq!(run.passes[1].passage(), Some(first_a));
    assert_eq!(run.passes[3].passage(), Some(second_a));
}

#[test]
fn double_trigger_is_debounced() {
    let mut log = Log::default();
    let start = log.pass(S, 10.0);
    let bounce = log.pass(S, 10.05); // 車体の隙間などで 50ms 後にもう一度反応
    log.pass(G, 70.0);
    let config = Config::new(Course::new(vec![S, G]).unwrap());

    let outcome = compute(&config, &log.events, None);
    assert_eq!(outcome.runs.len(), 1);
    assert_eq!(
        outcome.passage(bounce).unwrap().disposition,
        Disposition::Debounced { by: start }
    );
}

#[test]
fn correction_on_debounced_passage_goes_to_the_accepted_one() {
    // 遅れて届いた、より早い通過記録が二重反応の判定を入れ替えても、訂正は失われない。
    let mut log = Log {
        next_id: 10,
        ..Log::default()
    };
    let later = log.pass(S, 10.05);
    log.pass(G, 70.0);
    log.push(Event::SetCar {
        run: RunId(later),
        car: CarChoice::Car(CarId(9)),
    });
    log.next_id = 0;
    let earlier = log.pass(S, 10.0);

    let config = Config::new(Course::new(vec![S, G]).unwrap());
    let outcome = compute(&config, &log.events, None);
    assert_eq!(outcome.runs.len(), 1);
    assert_eq!(outcome.runs[0].id, RunId(earlier));
    assert_eq!(outcome.runs[0].car, Some(CarId(9)));
    assert!(outcome.warnings.is_empty());
}

#[test]
fn sensor_delay_is_corrected_per_gate() {
    let mut config = Config::new(Course::new(vec![S, G]).unwrap());
    config.gates.insert(
        G,
        GateConfig {
            delay: 12 * MILLISECOND,
            ..GateConfig::default()
        },
    );
    let mut log = Log::default();
    let start = log.pass(S, 10.0);
    log.pass(G, 70.012);

    let outcome = compute(&config, &log.events, None);
    assert_eq!(run_of(&outcome, start).total(), Some(s(60.0)));
}

#[test]
fn void_passage_is_ignored() {
    let mut log = Log::default();
    let noise = log.pass(S, 5.0); // 人がスタートの光電管を横切った
    let start = log.pass(S, 10.0);
    log.pass(G, 70.0);
    let config = Config::new(Course::new(vec![S, G]).unwrap());

    // 誤反応が走行になり、本当の走行のゴールを奪ってしまう。
    let outcome = compute(&config, &log.events, None);
    assert_eq!(outcome.runs.len(), 2);
    assert_eq!(run_of(&outcome, noise).total(), Some(s(65.0)));
    assert_eq!(run_of(&outcome, start).status, RunStatus::Running);

    log.push(Event::SetVoid {
        passage: noise,
        void: true,
    });
    let outcome = compute(&config, &log.events, None);
    assert_eq!(outcome.runs.len(), 1);
    assert_eq!(run_of(&outcome, start).total(), Some(s(60.0)));
    assert_eq!(
        outcome.passage(noise).unwrap().disposition,
        Disposition::Void
    );
}

#[test]
fn too_short_segment_is_not_assigned() {
    let mut log = Log::default();
    let start = log.pass(S, 10.0);
    let noise = log.pass(I1, 11.0); // スタートの 1 秒後: 最小区間タイム 5 秒に満たない
    let i1 = log.pass(I1, 30.0);
    log.pass(I2, 50.0);
    log.pass(G, 70.0);

    let outcome = compute(&gymkhana(), &log.events, None);
    assert_eq!(
        outcome.passage(noise).unwrap().disposition,
        Disposition::Orphan
    );
    assert_eq!(run_of(&outcome, start).passes[1].passage(), Some(i1));
}

#[test]
fn missed_intermediate_is_skipped_and_flagged() {
    let mut log = Log::default();
    let start = log.pass(S, 10.0);
    // I1 は反応しなかった
    log.pass(I2, 50.0);
    log.pass(G, 70.0);

    let outcome = compute(&gymkhana(), &log.events, None);
    let run = run_of(&outcome, start);
    assert_eq!(run.status, RunStatus::Finished);
    assert_eq!(run.passes[1], Slot::Missed);
    assert_eq!(run.total(), Some(s(60.0)));
    assert!(run.flags.contains(&RunFlag::MissingPass { position: 1 }));
}

#[test]
fn late_course_out_report_reassigns_passages() {
    // A がコースアウトしたが報告が遅れ、後続 B の通過が A に割り当てられていた。
    let mut log = Log::default();
    log.queue(1, 0.0);
    log.queue(2, 0.0);
    let a = log.pass(S, 10.0);
    log.pass(I1, 30.0);
    let b = log.pass(S, 35.0);
    // A は I1 の後でコースアウト
    log.pass(I1, 55.0);
    log.pass(I2, 75.0);
    log.pass(G, 95.0);

    let outcome = compute(&gymkhana(), &log.events, None);
    assert_eq!(
        run_of(&outcome, a).status,
        RunStatus::Finished,
        "wrong before the report"
    );
    assert_eq!(run_of(&outcome, b).status, RunStatus::Running);

    log.push(Event::ReportOff {
        run: RunId(a),
        last_position: 1,
        at: s(100.0),
    });
    let outcome = compute(&gymkhana(), &log.events, None);
    let run_a = run_of(&outcome, a);
    assert!(matches!(
        run_a.status,
        RunStatus::Off {
            cause: OffCause::Reported,
            ..
        }
    ));
    assert_eq!(run_a.result, RunResult::Dnf);
    assert_eq!(run_of(&outcome, b).total(), Some(s(60.0)));

    // 報告を取り消すと元に戻る。
    log.push(Event::CancelReport { run: RunId(a) });
    let outcome = compute(&gymkhana(), &log.events, None);
    assert_eq!(run_of(&outcome, a).status, RunStatus::Finished);
}

#[test]
fn reported_run_takes_no_passage_after_report_time() {
    // 本人は「I1 の後に出た」と言ったが、実は I1 も反応していなかった。報告時刻以降の通過は割り当てない。
    let mut log = Log::default();
    let a = log.pass(S, 10.0);
    log.push(Event::ReportOff {
        run: RunId(a),
        last_position: 1,
        at: s(25.0),
    });
    let b = log.pass(S, 35.0);
    let b_i1 = log.pass(I1, 55.0);

    let outcome = compute(&gymkhana(), &log.events, None);
    assert_eq!(
        run_of(&outcome, a).status,
        RunStatus::Off {
            cause: OffCause::Reported,
            at: s(25.0)
        }
    );
    assert_eq!(run_of(&outcome, b).passes[1].passage(), Some(b_i1));
}

#[test]
fn full_course_pushes_out_the_run_most_behind() {
    let mut config = gymkhana();
    config.max_on_course = Some(2);
    let mut log = Log::default();
    // 基準になる走行
    log.pass(S, 0.0);
    log.pass(I1, 20.0);
    log.pass(I2, 40.0);
    log.pass(G, 60.0);
    // A は I1 の後でコースアウト（報告なし）
    let a = log.pass(S, 100.0);
    log.pass(I1, 120.0);
    let b = log.pass(S, 125.0);
    log.pass(I1, 145.0); // b
    // ここで満杯。A は I1 から 50 秒（基準 20 秒の 2.5 倍）、B は I1 から 25 秒（1.25 倍）。
    let c = log.pass(S, 170.0);
    log.pass(I2, 171.0); // b（A が押し出されていなければ A に割り当てられてしまう）

    let outcome = compute(&config, &log.events, None);
    let run_a = run_of(&outcome, a);
    assert_eq!(
        run_a.status,
        RunStatus::Off {
            cause: OffCause::PushedOut { by: RunId(c) },
            at: s(170.0)
        }
    );
    assert!(run_a.needs_review());
    assert_eq!(run_of(&outcome, b).split(2), Some(s(46.0)));
}

#[test]
fn run_exceeding_max_time_is_timed_out() {
    let mut config = gymkhana();
    config.max_run_time = Some(s(180.0));
    let mut log = Log::default();
    let start = log.pass(S, 10.0);

    let outcome = compute(&config, &log.events, Some(s(100.0)));
    assert_eq!(run_of(&outcome, start).status, RunStatus::Running);

    let outcome = compute(&config, &log.events, Some(s(200.0)));
    let run = run_of(&outcome, start);
    assert_eq!(
        run.status,
        RunStatus::Off {
            cause: OffCause::TimedOut,
            at: s(190.0)
        }
    );
    assert!(run.status.is_estimated());
}

#[test]
fn penalties_and_standings() {
    let config = Config::new(Course::new(vec![S, G]).unwrap());
    let mut log = Log::default();
    for car in [1, 2, 1, 2] {
        log.queue(car, 0.0);
    }
    let r1 = log.pass(S, 0.0);
    log.pass(G, 60.0);
    let r2 = log.pass(S, 100.0);
    log.pass(G, 158.0);
    let r3 = log.pass(S, 200.0);
    log.pass(G, 259.0);
    let r4 = log.pass(S, 300.0);
    log.pass(G, 357.0);
    log.push(Event::SetPenalty {
        run: RunId(r2),
        penalty: Penalty {
            pylons: 1,
            ..Penalty::default()
        },
    });
    log.push(Event::SetPenalty {
        run: RunId(r4),
        penalty: Penalty {
            miss_course: true,
            ..Penalty::default()
        },
    });

    let outcome = compute(&config, &log.events, None);
    assert_eq!(run_of(&outcome, r1).result, RunResult::Time(s(60.0)));
    assert_eq!(run_of(&outcome, r2).result, RunResult::Time(s(63.0)));
    assert_eq!(run_of(&outcome, r4).result, RunResult::MissCourse);
    let standings = outcome.standings();
    assert_eq!(standings[0].car, CarId(1));
    assert_eq!(standings[0].best, Some((RunId(r3), s(59.0))));
    assert_eq!(standings[1].car, CarId(2));
    assert_eq!(standings[1].best, Some((RunId(r2), s(63.0))));
    assert_eq!(standings[1].runs, 2);
}

#[test]
fn warns_about_corrections_without_run() {
    let config = Config::new(Course::new(vec![S, G]).unwrap());
    let mut log = Log::default();
    log.pass(S, 0.0);
    let finish = log.pass(G, 60.0);
    log.push(Event::SetCar {
        run: RunId(finish),
        car: CarChoice::Car(CarId(1)),
    });
    log.push(Event::SetVoid {
        passage: PassageId(99),
        void: true,
    });

    let outcome = compute(&config, &log.events, None);
    assert!(
        outcome
            .warnings
            .contains(&Warning::NoSuchRun(RunId(finish)))
    );
    assert!(
        outcome
            .warnings
            .contains(&Warning::UnknownPassage(PassageId(99)))
    );
}

#[test]
fn result_does_not_depend_on_arrival_order_of_passages() {
    let mut log = Log::default();
    for car in 1..=2 {
        log.queue(car, 0.0);
    }
    log.pass(S, 10.0);
    log.pass(I1, 30.0);
    log.pass(S, 31.0);
    log.pass(I2, 50.0);
    log.pass(I1, 51.0);
    log.pass(G, 70.0);
    log.pass(I2, 71.0);
    log.pass(G, 91.0);
    let in_order = compute(&gymkhana(), &log.events, None);

    let mut shuffled = log.events.clone();
    shuffled[2..].reverse();
    let reordered = compute(&gymkhana(), &shuffled, None);
    assert_eq!(in_order.runs, reordered.runs);
}

#[test]
fn run_exceeding_max_segment_time_is_timed_out() {
    // スタートとゴールが同じ光電管。A はコースアウトしたが報告がなく、B のスタートが A のゴールに見えてしまう。
    let sg = GateId(0);
    let mut config = Config::new(Course::new(vec![sg, sg]).unwrap());
    config.min_segment = vec![s(40.0)];
    let mut log = Log::default();
    let a = log.pass(sg, 0.0);
    let b = log.pass(sg, 100.0);
    log.pass(sg, 160.0);

    let outcome = compute(&config, &log.events, None);
    assert_eq!(
        run_of(&outcome, a).total(),
        Some(s(100.0)),
        "wrong without max segment"
    );

    config.max_segment = vec![Some(s(90.0))];
    let outcome = compute(&config, &log.events, None);
    assert_eq!(
        run_of(&outcome, a).status,
        RunStatus::Off {
            cause: OffCause::TimedOut,
            at: s(90.0)
        }
    );
    assert_eq!(run_of(&outcome, b).total(), Some(s(60.0)));
}

#[test]
fn contradiction_reassigns_a_stolen_intermediate() {
    // A の I2 が反応せず、後続 B の I2 が A に割り当てられる。直後の A のゴールは、A の I2 から
    // 最小区間タイムが経っていないので矛盾する。B の I2 を B に付け替え、A のゴールを A に割り当てる。
    let mut log = Log::default();
    let a = log.pass(S, 0.0);
    log.pass(I1, 20.0);
    let b = log.pass(S, 16.0);
    log.pass(I1, 36.0);
    // A の I2（40 秒）は反応しなかった
    let b_i2 = log.pass(I2, 61.0);
    let a_g = log.pass(G, 63.0);
    let b_g = log.pass(G, 81.0);

    let outcome = compute(&gymkhana(), &log.events, None);
    let run_a = run_of(&outcome, a);
    let run_b = run_of(&outcome, b);
    assert_eq!(run_a.passes[3].passage(), Some(a_g));
    assert_eq!(run_a.passes[2], Slot::Missed);
    assert_eq!(run_b.passes[2].passage(), Some(b_i2));
    assert_eq!(run_b.passes[3].passage(), Some(b_g));
    assert!(run_a.flags.contains(&RunFlag::Reassigned { position: 2 }));
    assert!(run_b.flags.contains(&RunFlag::Reassigned { position: 2 }));
    assert_eq!(
        outcome.passage(b_i2).unwrap().disposition,
        Disposition::Assigned {
            run: RunId(b),
            position: 2
        }
    );
}

#[test]
fn older_run_skipping_a_missed_gate_is_preferred_over_repair() {
    // A は I2 が抜けたままゴールへ。B は I2 を通過した直後。A のゴールは A に（B の I2 は付け替えない）。
    let mut log = Log::default();
    let a = log.pass(S, 0.0);
    log.pass(I1, 20.0);
    let b = log.pass(S, 16.0);
    log.pass(I1, 36.0);
    let b_i2 = log.pass(I2, 59.0);
    let a_g = log.pass(G, 60.0);

    let outcome = compute(&gymkhana(), &log.events, None);
    // 最も古い A が I2 を受け取る（ここは見分けられない）が、A のゴールの矛盾で付け替わる。
    assert_eq!(run_of(&outcome, a).passes[3].passage(), Some(a_g));
    assert_eq!(run_of(&outcome, b).passes[2].passage(), Some(b_i2));
}
