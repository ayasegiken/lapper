//! JSON との変換（`serde` 機能）。保存したログと、画面に配る結果の形を確かめる。
#![cfg(feature = "serde")]

use lapper_core::*;
use serde_json::json;

const S: GateId = GateId(0);
const G: GateId = GateId(1);

fn sample_log() -> Vec<Event> {
    let pass = |id, gate, time| {
        Event::Passage(Passage {
            id: PassageId(id),
            gate,
            time,
            source: PassageSource::Sensor,
        })
    };
    vec![
        Event::Enqueue {
            car: CarId(7),
            at: 0,
        },
        pass(1, S, 10 * SECOND),
        pass(2, G, 72 * SECOND),
        Event::SetCar {
            run: RunId(PassageId(1)),
            car: CarChoice::Car(CarId(8)),
        },
        Event::SetPenalty {
            run: RunId(PassageId(1)),
            penalty: Penalty {
                pylons: 1,
                ..Penalty::default()
            },
        },
    ]
}

#[test]
fn log_round_trips_through_json() {
    let log = sample_log();
    let text = serde_json::to_string(&log).unwrap();
    let back: Vec<Event> = serde_json::from_str(&text).unwrap();
    assert_eq!(back, log);
}

#[test]
fn events_are_tagged_with_type() {
    let log = sample_log();
    assert_eq!(
        serde_json::to_value(&log[1]).unwrap(),
        json!({ "type": "Passage", "id": 1, "gate": 0, "time": 10_000_000, "source": "Sensor" })
    );
    assert_eq!(
        serde_json::to_value(&log[3]).unwrap(),
        json!({ "type": "SetCar", "run": 1, "car": { "type": "Car", "car": 8 } })
    );
}

#[test]
fn config_round_trips_through_json() {
    let mut config = Config::new(Course::new(vec![S, G]).unwrap());
    config.gates.insert(
        G,
        GateConfig {
            delay: 8 * MILLISECOND,
            lockout: 500 * MILLISECOND,
        },
    );
    config.max_on_course = Some(4);
    let text = serde_json::to_string(&config).unwrap();
    let back: Config = serde_json::from_str(&text).unwrap();
    assert_eq!(back, config);
}

#[test]
fn course_from_json_is_validated() {
    assert_eq!(
        serde_json::from_value::<Course>(json!([0, 0])).unwrap(),
        Course::new(vec![S, S]).unwrap()
    );
    assert!(serde_json::from_value::<Course>(json!([0])).is_err());
}

#[test]
fn outcome_serializes_results() {
    let config = Config::new(Course::new(vec![S, G]).unwrap());
    let outcome = compute(&config, &sample_log(), None);
    let value = serde_json::to_value(&outcome).unwrap();
    let run = &value["runs"][0];
    assert_eq!(run["car"], json!(8));
    assert_eq!(run["result"], json!({ "type": "Time", "time": 67_000_000 }));
    assert_eq!(run["status"], json!({ "type": "Finished" }));
    assert_eq!(
        value["passages"][1]["disposition"],
        json!({ "type": "Assigned", "run": 1, "position": 1 })
    );
}
