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
