use crate::types::{CarId, GateId, Micros, PassageId, RunId};

/// 計測ロジックへの入力。追記だけのログとして保存し、書き換えない。
///
/// 訂正も新しいイベントとして追記する。同じ対象への訂正は、ログで後にあるものが優先される。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// 光電管の通過記録（または手動で入れた通過）。
    Passage(Passage),
    /// 通過記録を無効にする（`void: true`）か、無効を取り消す（`void: false`）。
    /// 人がゲートを横切った、などの誤反応に使う。
    SetVoid { passage: PassageId, void: bool },
    /// 出走待ちの列の最後に車を入れる。`at` はコースの時計での時刻。
    Enqueue { car: CarId, at: Micros },
    /// 出走待ちの列から車を外す（列の中で最初に見つかったもの）。
    Dequeue { car: CarId, at: Micros },
    /// 走行の車を指定する。出走待ちの列からの自動の割り当てより優先する。
    SetCar { run: RunId, car: CarChoice },
    /// コースアウト / リタイアの報告。
    ///
    /// `last_position` はその走行が最後に通過したポジション。走行はこれより先のポジションには割り当てられず、
    /// `at`（報告した時刻）より後の通過にも割り当てられない。
    ReportOff {
        run: RunId,
        last_position: usize,
        at: Micros,
    },
    /// コースアウト / リタイアの報告を取り消す。
    CancelReport { run: RunId },
    /// ペナルティを設定する（加算ではなく置き換え）。
    SetPenalty { run: RunId, penalty: Penalty },
}

/// 通過記録。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Passage {
    pub id: PassageId,
    pub gate: GateId,
    /// 計測ボックスが打刻した時刻（遅延の補正前）。
    pub time: Micros,
    pub source: PassageSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassageSource {
    Sensor,
    /// 運営が手で入れた通過（光電管が反応しなかったときなど）。
    Manual,
}

/// 走行の車の指定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CarChoice {
    /// 出走待ちの列から自動で割り当てる（指定の取り消し）。
    Auto,
    /// この車にする。
    Car(CarId),
    /// 車なしにする（試走や誤反応など）。
    NoCar,
}

/// ペナルティ。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Penalty {
    /// パイロンタッチの回数。
    pub pylons: u32,
    /// ミスコース。
    pub miss_course: bool,
    /// 完走していても未完走扱いにする。
    pub dnf: bool,
}
