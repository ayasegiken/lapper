use std::collections::BTreeMap;

use crate::course::Course;
use crate::types::{GateId, MILLISECOND, Micros, SECOND};

/// ゲート（チャンネル）ごとの設定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateConfig {
    /// センサーの反応の遅れ。打刻された時刻からこの値を引いた時刻を通過時刻とする。
    pub delay: Micros,
    /// 一度反応したら、この時間内の同じゲートの反応を無視する（二重反応の防止）。
    pub lockout: Micros,
}

impl Default for GateConfig {
    fn default() -> Self {
        GateConfig {
            delay: 0,
            lockout: 500 * MILLISECOND,
        }
    }
}

/// 計測ロジックの設定。
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub course: Course,
    /// ゲートごとの設定。ないゲートは `default_gate` を使う。
    pub gates: BTreeMap<GateId, GateConfig>,
    pub default_gate: GateConfig,
    /// 区間ごとの最小タイム。これより短い区間タイムになる割り当てはしない。
    /// 長さは `course.segments()`。足りない分は 0 とみなす。
    pub min_segment: Vec<Micros>,
    /// 区間ごとの最大タイム。最後の通過からこの時間内に次のポジションを通過しない走行は、時間切れとして外す。
    /// 中間の反応が抜けると次の通過までが長くなるので、余裕を持たせて設定する。
    /// 長さは `course.segments()`。足りない分は制限なしとみなす。
    pub max_segment: Vec<Option<Micros>>,
    /// 同時にコース上にいられる最大の台数。満杯のときにスタートがあると 1 台を押し出す。
    pub max_on_course: Option<usize>,
    /// 最大走行時間。これを超えた走行は未完走（時間切れ）とする。
    pub max_run_time: Option<Micros>,
    /// パイロンタッチ 1 回あたりの加算タイム。
    pub pylon_penalty: Micros,
    /// 区間タイムが全走行の中央値のこの割合（%）を超えたら「遅すぎる区間」として目印を付ける。
    pub slow_segment_percent: Option<u32>,
    /// 区間タイムが全走行の中央値のこの割合（%）を下回ったら「速すぎる区間」として目印を付ける。
    pub fast_segment_percent: Option<u32>,
}

impl Config {
    /// 既定値の設定を作る。区間タイムの制限なし、同時走行と最大走行時間は無制限。
    pub fn new(course: Course) -> Config {
        let segments = course.segments();
        Config {
            course,
            gates: BTreeMap::new(),
            default_gate: GateConfig::default(),
            min_segment: vec![0; segments],
            max_segment: vec![None; segments],
            max_on_course: None,
            max_run_time: None,
            pylon_penalty: 5 * SECOND,
            slow_segment_percent: Some(150),
            fast_segment_percent: Some(80),
        }
    }

    pub fn gate(&self, gate: GateId) -> GateConfig {
        self.gates.get(&gate).copied().unwrap_or(self.default_gate)
    }

    pub fn min_segment(&self, segment: usize) -> Micros {
        self.min_segment.get(segment).copied().unwrap_or(0)
    }

    pub fn max_segment(&self, segment: usize) -> Option<Micros> {
        self.max_segment.get(segment).copied().flatten()
    }

    /// ポジション `from` から `to` までの最小タイムの合計。
    pub fn min_between(&self, from: usize, to: usize) -> Micros {
        (from..to).map(|s| self.min_segment(s)).sum()
    }
}
