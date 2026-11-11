//! API でやり取りする型。`cargo test` で TypeScript の型定義を `web/src/lib/bindings/` に書き出す。

use std::collections::BTreeMap;

use lapper_core::{Config, Course, Event, GateId, Micros, Outcome};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// セッション。1 回の計測の単位（ヒートや練習会の 1 枠）で、設定とイベントのログを持つ。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SessionInfo {
    pub id: i64,
    pub name: String,
    pub config: Config,
    /// ゲートの表示名（`S`、`I1`、`G` など）。
    pub gate_names: BTreeMap<GateId, String>,
    /// 作った時刻（UNIX 時刻、ミリ秒）。
    pub created_at: i64,
}

/// セッションを作る要求。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NewSession {
    pub name: String,
    pub config: Config,
    /// ゲートの表示名。省略するとコースから決める（[`default_gate_names`]）。
    #[serde(default)]
    pub gate_names: Option<BTreeMap<GateId, String>>,
}

/// イベントを追記する要求。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AppendEvents {
    pub events: Vec<Event>,
}

/// 追記の結果。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Appended {
    /// 追記後のログのイベントの数。
    pub events: usize,
}

/// コースの時計（計測ボックスの時計）の現在の時刻。入力元が定期的に送る。
///
/// 最大走行時間などの時間切れは、この時刻を基準に判断する。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ClockSync {
    pub time: Micros,
}

/// あるセッションの、ある時点の計算結果。結果が変わるたびに SSE で配る。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Snapshot {
    pub session: i64,
    /// ログのイベントの数。
    pub events: usize,
    pub outcome: Outcome,
}

/// コースからゲートの表示名を決める。
///
/// スタートだけのゲートは `S`、ゴールだけは `G`、両方なら `SG`、それ以外は最初に通る順に `I1`、`I2`、…。
pub fn default_gate_names(course: &Course) -> BTreeMap<GateId, String> {
    let finish = course.finish();
    let mut names = BTreeMap::new();
    let mut intermediates = 0;
    for &gate in course.gates() {
        if names.contains_key(&gate) {
            continue;
        }
        let at = |position: usize| course.gate_at(position) == gate;
        let name = match (at(0), at(finish)) {
            (true, true) => "SG".to_string(),
            (true, false) => "S".to_string(),
            (false, true) => "G".to_string(),
            (false, false) => {
                intermediates += 1;
                format!("I{intermediates}")
            }
        };
        names.insert(gate, name);
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(gates: &[u16]) -> Vec<String> {
        let course = Course::new(gates.iter().map(|&g| GateId(g)).collect()).unwrap();
        default_gate_names(&course).into_values().collect()
    }

    #[test]
    fn names_gates_by_their_role() {
        assert_eq!(names(&[0, 1, 2, 3]), ["S", "I1", "I2", "G"]);
        assert_eq!(names(&[0, 0]), ["SG"]);
        assert_eq!(names(&[0, 1, 2, 1, 3]), ["S", "I1", "I2", "G"]);
    }
}
