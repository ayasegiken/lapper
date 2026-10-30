//! 計測ロジックの結果を、実際の走行（正解）と照らし合わせる。

use std::ops::AddAssign;

use lapper_core::{Disposition, Outcome, Run, RunId, RunStatus};

use crate::generate::{Simulation, TruthRun};

/// 実際の走行 1 回ごとの判定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// 完走して、タイムと車が正しい。
    Correct,
    /// コースアウトして、未完走になっている。
    CorrectDnf,
    /// タイム（または未完走）は正しいが、車が違う。
    WrongCar,
    /// 完走したが、違うゴールの通過が割り当てられている。
    WrongTime,
    /// 完走したが、未完走や走行中になっている。
    NoTime,
    /// コースアウトしたのに、完走になっている。
    FalseFinish,
    /// 走行として認識されていない（スタートの反応の抜けなど）。
    Lost,
}

impl Verdict {
    pub fn is_error(self) -> bool {
        !matches!(self, Verdict::Correct | Verdict::CorrectDnf)
    }
}

/// 判定の詳細。
#[derive(Debug, Clone)]
pub struct Judged {
    pub truth: usize,
    pub run: Option<RunId>,
    pub verdict: Verdict,
    /// 計測ロジックがその走行に確認を促していたか。
    pub flagged: bool,
}

/// 集計。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tally {
    pub runs: usize,
    pub finished: usize,
    pub off: usize,
    pub correct: usize,
    pub correct_dnf: usize,
    pub wrong_car: usize,
    pub wrong_time: usize,
    pub no_time: usize,
    pub false_finish: usize,
    pub lost: usize,
    /// タイムの誤り（違うタイム、完走したのにタイムなし、コースアウトしたのに完走）のうち、確認を促す目印がなかったもの。
    pub silent_time: usize,
    /// 車の誤りのうち、確認を促す目印がなかったもの。
    pub silent_car: usize,
    /// 実際の走行に対応しない走行（誤反応から始まった走行など）。
    pub phantom: usize,
    /// 中間計時の割り当ての誤り。
    pub split_errors: usize,
    /// 確認を促す目印が付いた走行。
    pub flagged: usize,
}

impl AddAssign for Tally {
    fn add_assign(&mut self, o: Tally) {
        self.runs += o.runs;
        self.finished += o.finished;
        self.off += o.off;
        self.correct += o.correct;
        self.correct_dnf += o.correct_dnf;
        self.wrong_car += o.wrong_car;
        self.wrong_time += o.wrong_time;
        self.no_time += o.no_time;
        self.false_finish += o.false_finish;
        self.lost += o.lost;
        self.silent_time += o.silent_time;
        self.silent_car += o.silent_car;
        self.phantom += o.phantom;
        self.split_errors += o.split_errors;
        self.flagged += o.flagged;
    }
}

impl Tally {
    pub fn errors(&self) -> usize {
        self.wrong_car + self.wrong_time + self.no_time + self.false_finish + self.lost
    }
}

pub struct Evaluation {
    pub judged: Vec<Judged>,
    pub tally: Tally,
}

pub fn evaluate(sim: &Simulation, outcome: &Outcome) -> Evaluation {
    let mut tally = Tally {
        runs: sim.truth.len(),
        ..Tally::default()
    };
    let mut matched: Vec<RunId> = Vec::new();
    let mut judged = Vec::new();

    for (i, truth) in sim.truth.iter().enumerate() {
        if truth.finished() {
            tally.finished += 1;
        } else {
            tally.off += 1;
        }
        let run = find_run(outcome, truth);
        if let Some(run) = run {
            matched.push(run.id);
            tally.split_errors += split_errors(truth, run);
        }
        let verdict = judge(truth, run);
        let flagged = run.is_some_and(Run::needs_review);
        match verdict {
            Verdict::Correct => tally.correct += 1,
            Verdict::CorrectDnf => tally.correct_dnf += 1,
            Verdict::WrongCar => tally.wrong_car += 1,
            Verdict::WrongTime => tally.wrong_time += 1,
            Verdict::NoTime => tally.no_time += 1,
            Verdict::FalseFinish => tally.false_finish += 1,
            Verdict::Lost => tally.lost += 1,
        }
        if !flagged {
            match verdict {
                Verdict::WrongTime | Verdict::NoTime | Verdict::FalseFinish => {
                    tally.silent_time += 1
                }
                Verdict::WrongCar => tally.silent_car += 1,
                _ => {}
            }
        }
        judged.push(Judged {
            truth: i,
            run: run.map(|r| r.id),
            verdict,
            flagged,
        });
    }

    tally.phantom = outcome
        .runs
        .iter()
        .filter(|r| !matched.contains(&r.id))
        .count();
    tally.flagged = outcome.runs.iter().filter(|r| r.needs_review()).count();
    Evaluation { judged, tally }
}

/// 実際の走行のスタートの通過から始まった走行を探す。二重反応として無視されていたら、無視させた通過をたどる。
fn find_run<'a>(outcome: &'a Outcome, truth: &TruthRun) -> Option<&'a Run> {
    let mut id = truth.passages[0]?;
    loop {
        match outcome.passage(id)?.disposition {
            Disposition::Assigned { run, position: 0 } => return outcome.run(run),
            Disposition::Debounced { by } => id = by,
            _ => return None,
        }
    }
}

fn judge(truth: &TruthRun, run: Option<&Run>) -> Verdict {
    let Some(run) = run else {
        return Verdict::Lost;
    };
    let car_ok = run.car == Some(truth.car);
    let finish = truth.passages.len() - 1;
    match (truth.finished(), run.status) {
        (true, RunStatus::Finished) => {
            if run.passes[finish].passage() != truth.passages[finish] {
                Verdict::WrongTime
            } else if !car_ok {
                Verdict::WrongCar
            } else {
                Verdict::Correct
            }
        }
        (true, _) => Verdict::NoTime,
        (false, RunStatus::Finished) => Verdict::FalseFinish,
        (false, _) if !car_ok => Verdict::WrongCar,
        (false, _) => Verdict::CorrectDnf,
    }
}

fn split_errors(truth: &TruthRun, run: &Run) -> usize {
    let finish = truth.passages.len() - 1;
    (1..finish)
        .filter(|&q| truth.passages[q].is_some() && run.passes[q].passage() != truth.passages[q])
        .count()
}
