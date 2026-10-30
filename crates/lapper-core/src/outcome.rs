use crate::event::{PassageSource, Penalty};
use crate::types::{CarId, GateId, Micros, PassageId, RunId};

/// ログから計算した結果。ログが同じなら常に同じ結果になる。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// 走行。スタートの時刻順。
    pub runs: Vec<Run>,
    /// 通過記録。補正後の時刻順。
    pub passages: Vec<PassageReport>,
    /// 出走待ちの列（先頭から）。
    pub queue: Vec<CarId>,
    pub warnings: Vec<Warning>,
}

/// 1 回の走行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub id: RunId,
    pub car: Option<CarId>,
    pub car_source: CarSource,
    pub start_time: Micros,
    /// ポジションごとの通過。長さはコースのポジションの数。
    pub passes: Vec<Slot>,
    pub status: RunStatus,
    pub penalty: Penalty,
    pub result: RunResult,
    /// 確認を促す目印。
    pub flags: Vec<RunFlag>,
}

/// 走行の車がどう決まったか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CarSource {
    /// 出走待ちの列から自動で割り当てた。
    Queue,
    /// `SetCar` で指定された。
    Manual,
    /// 車がいない（未割当の走行）。
    Unassigned,
}

/// ポジションの通過状況。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Passed {
        passage: PassageId,
        time: Micros,
    },
    /// 後のポジションを通過しているのに、ここの通過記録がない（反応の抜け）。
    Missed,
    NotReached,
}

impl Slot {
    pub fn time(&self) -> Option<Micros> {
        match self {
            Slot::Passed { time, .. } => Some(*time),
            _ => None,
        }
    }

    pub fn passage(&self) -> Option<PassageId> {
        match self {
            Slot::Passed { passage, .. } => Some(*passage),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Running,
    Finished,
    /// コースから外れた。`at` は計算の上で外れたとみなした時刻。
    Off {
        cause: OffCause,
        at: Micros,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OffCause {
    /// コースアウト / リタイアの報告による。
    Reported,
    /// コースが満杯のときに次の車がスタートしたので押し出した（推定）。
    PushedOut { by: RunId },
    /// 最大走行時間を超えた（推定）。
    TimedOut,
}

impl RunStatus {
    /// 報告ではなく推定で決めた状態か。画面で目立たせ、確認を促す。
    pub fn is_estimated(&self) -> bool {
        matches!(
            self,
            RunStatus::Off {
                cause: OffCause::PushedOut { .. } | OffCause::TimedOut,
                ..
            }
        )
    }
}

/// 走行の結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunResult {
    Running,
    /// ペナルティを加えたタイム。
    Time(Micros),
    MissCourse,
    Dnf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunFlag {
    /// 通過記録の抜け（そのポジションを飛ばして割り当てた）。
    MissingPass { position: usize },
    /// 区間タイムが全走行の中央値に比べて長すぎる。取り違えの可能性がある。
    SlowSegment {
        segment: usize,
        time: Micros,
        median: Micros,
    },
    /// 後から来た通過との矛盾から、このポジションの割り当てを付け替えた（この走行から外した、またはこの走行に移した）。
    Reassigned { position: usize },
    /// 区間タイムが全走行の中央値に比べて短すぎる。別の車の通過を割り当てた可能性がある。
    FastSegment {
        segment: usize,
        time: Micros,
        median: Micros,
    },
    /// 車が決まっていない。
    NoCar,
}

impl Run {
    /// スタートからゴールまでのタイム（ペナルティなし）。
    pub fn total(&self) -> Option<Micros> {
        match self.status {
            RunStatus::Finished => self.split(self.passes.len() - 1),
            _ => None,
        }
    }

    /// スタートからポジション `position` までのタイム。
    pub fn split(&self, position: usize) -> Option<Micros> {
        Some(self.passes.get(position)?.time()? - self.start_time)
    }

    /// 区間 `segment`（ポジション `segment` から `segment + 1`）のタイム。
    pub fn segment(&self, segment: usize) -> Option<Micros> {
        let from = self.passes.get(segment)?.time()?;
        let to = self.passes.get(segment + 1)?.time()?;
        Some(to - from)
    }

    /// 確認が必要な走行か（推定による状態、または目印がある）。
    pub fn needs_review(&self) -> bool {
        self.status.is_estimated() || !self.flags.is_empty()
    }
}

/// 通過記録とその扱い。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PassageReport {
    pub id: PassageId,
    pub gate: GateId,
    /// 打刻された時刻（補正前）。
    pub raw_time: Micros,
    /// 遅延を補正した時刻。
    pub time: Micros,
    pub source: PassageSource,
    pub disposition: Disposition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    Assigned {
        run: RunId,
        position: usize,
    },
    /// 同じゲートの直前の反応から間もないので無視した。
    Debounced {
        by: PassageId,
    },
    /// `SetVoid` で無効にされた。
    Void,
    /// どの走行にも割り当てられなかった。誤反応か、割り当ての誤りの可能性がある。
    Orphan,
}

/// 計算中に見つかった、入力の問題。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Warning {
    /// 同じ ID の通過記録が複数ある。最初のものだけを使った。
    DuplicatePassage(PassageId),
    /// 訂正が、存在しない通過記録を指している。
    UnknownPassage(PassageId),
    /// 訂正が指している走行がない（そのスタートの通過が無効にされた、走行にならなかった、など）。
    NoSuchRun(RunId),
    /// コースアウトの報告のポジションがゴール以降。報告を無視した。
    InvalidReport(RunId),
    /// 出走待ちの列にいない車を外そうとした。
    NotQueued(CarId),
}

/// 車ごとのベストタイム。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Standing {
    pub car: CarId,
    /// ベストタイムの走行とタイム（ペナルティ込み）。
    pub best: Option<(RunId, Micros)>,
    /// 走行の数。
    pub runs: usize,
}

impl Outcome {
    pub fn run(&self, id: RunId) -> Option<&Run> {
        self.runs.iter().find(|r| r.id == id)
    }

    pub fn passage(&self, id: PassageId) -> Option<&PassageReport> {
        self.passages.iter().find(|p| p.id == id)
    }

    /// どの走行にも割り当てられなかった通過記録。
    pub fn orphans(&self) -> impl Iterator<Item = &PassageReport> {
        self.passages
            .iter()
            .filter(|p| p.disposition == Disposition::Orphan)
    }

    /// 車ごとのベストタイム。タイムのある車をタイム順に、その後にタイムのない車を車の順に並べる。
    pub fn standings(&self) -> Vec<Standing> {
        let mut by_car: Vec<Standing> = Vec::new();
        for run in &self.runs {
            let Some(car) = run.car else { continue };
            let index = match by_car.iter().position(|s| s.car == car) {
                Some(i) => i,
                None => {
                    by_car.push(Standing {
                        car,
                        best: None,
                        runs: 0,
                    });
                    by_car.len() - 1
                }
            };
            let standing = &mut by_car[index];
            standing.runs += 1;
            if let RunResult::Time(t) = run.result
                && standing.best.is_none_or(|(_, best)| t < best)
            {
                standing.best = Some((run.id, t));
            }
        }
        by_car.sort_by_key(|s| match s.best {
            Some((run, t)) => (0, t, run.0.0, 0),
            None => (1, 0, 0, s.car.0),
        });
        by_car
    }
}
