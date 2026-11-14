use lapper_core::GateId;
use lapper_protocol::inputs::MAX_INTERMEDIATES;
use lapper_sim::{Faults, PRESETS, Scenario, Tally, simulate};

fn sweep(preset: &str, faults: Faults, seeds: u64) -> Tally {
    let scenario = Scenario::preset(preset, faults).unwrap();
    let mut total = Tally::default();
    for seed in 1..=seeds {
        total += simulate(&scenario, seed).2.tally;
    }
    total
}

#[test]
fn every_run_is_correct_without_faults() {
    for preset in PRESETS {
        let t = sweep(preset, Faults::NONE, 20);
        assert_eq!(t.correct, t.runs, "{preset}: {t:?}");
        assert_eq!(t.phantom, 0, "{preset}: {t:?}");
    }
}

#[test]
fn time_errors_stay_rare_and_mostly_flagged_with_light_faults() {
    for preset in PRESETS {
        let t = sweep(preset, Faults::LIGHT, 20);
        let time_errors = t.wrong_time + t.no_time + t.false_finish;
        assert!(time_errors * 100 <= t.runs * 3, "{preset}: {t:?}");
        assert!(t.silent_time * 100 <= t.runs * 2, "{preset}: {t:?}");
    }
}

#[test]
fn intermediates_can_be_changed_keeping_the_lap() {
    let busy = Scenario::preset("busy", Faults::NONE).unwrap();
    let one = busy.clone().with_intermediates(1);
    assert_eq!(one.course, [GateId(0), GateId(1), GateId(3)]);
    assert_eq!(one.segment_times, [20.0, 45.0]);
    let none = busy.clone().with_intermediates(0);
    assert_eq!(none.course, [GateId(0), GateId(3)]);
    assert_eq!(none.segment_times, [65.0]);

    let long = Scenario::preset("long", Faults::NONE).unwrap();
    let two = long.with_intermediates(2);
    assert_eq!(two.course, [GateId(0), GateId(1), GateId(2), GateId(3)]);
    assert_eq!(two.segment_times, [55.0, 32.5, 32.5]);

    // スタートとゴールが同じ光電管なら、ゴールも 0 のまま
    let single = Scenario::preset("single", Faults::NONE).unwrap();
    assert_eq!(
        single.with_intermediates(1).course,
        [GateId(0), GateId(1), GateId(0)]
    );
}

#[test]
fn every_run_is_correct_without_faults_for_any_number_of_intermediates() {
    for preset in PRESETS {
        for n in 0..=MAX_INTERMEDIATES {
            let scenario = Scenario::preset(preset, Faults::NONE)
                .unwrap()
                .with_intermediates(n);
            let mut total = Tally::default();
            for seed in 1..=10 {
                total += simulate(&scenario, seed).2.tally;
            }
            assert_eq!(total.correct, total.runs, "{preset} / {n}: {total:?}");
        }
    }
}
