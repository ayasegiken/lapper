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
///
/// 送り手は、イベントごとに自分の中で重ならない番号を付ける。サーバーは送り手と番号の組で重複を除くので、
/// 届いたか分からないときは、同じ番号のまま送り直せばよい。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AppendEvents {
    /// 送り手の ID。計測ボックスなら機器ごとの ID、ブラウザーならページを開くたびに作るランダムな ID。
    pub source: String,
    pub events: Vec<NumberedEvent>,
}

/// 送り手が番号を付けたイベント。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NumberedEvent {
    /// 送り手の中での番号（0 〜 2^63 - 1）。連続していなくてもよい。
    pub seq: u64,
    pub event: Event,
    /// `true` なら、イベントの時刻（`Enqueue`、`Dequeue`、`ReportOff` の `at`、`Passage` の `time`）を、
    /// サーバーが今のコースの時刻で置き換える。コースの時計を知らない送り手（ブラウザー）が、
    /// 運営の操作や手動の通過を「今」のこととして送るときに使う。JSON では省略できる（省略すると `false`）。
    #[serde(default)]
    pub at_now: bool,
    /// `true` なら、`Passage` の `id` を、サーバーがまだ使われていない ID で置き換える。
    /// 計測ボックスの入力や手動の通過のように、送り手がセッションの中で一意な ID を決められないときに使う。
    /// JSON では省略できる（省略すると `false`）。
    #[serde(default)]
    pub new_id: bool,
    /// `at_now` のとき、操作してからこの要求を送るまでにかかった時間（ミリ秒）。サーバーは今の時刻からこの分を戻す。
    /// 送り直したときに、送り直した時刻ではなく操作した時刻にするため。JSON では省略できる（省略すると 0）。
    #[serde(default)]
    pub age_ms: u32,
}

/// 追記の結果。
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Appended {
    /// 新しく追記したイベントの数。届き済みの番号のイベントは数えない。
    pub accepted: usize,
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
