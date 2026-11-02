//! lapper の計測ロジックのシミュレーター。
//!
//! シナリオ（コース、台数、同時走行、障害の起きやすさ）から、実際に起きたこと（正解）と、
//! 計測ロジックに届くログ（通過記録、出走待ちの列、運営の訂正、コースアウトの報告）を作り、
//! 計測ロジックの結果を正解と照らし合わせる。乱数はシードで固定するので、同じシードなら同じ結果になる。

pub mod evaluate;
pub mod generate;
pub mod rng;
pub mod scenario;

pub use evaluate::{Evaluation, Judged, Tally, Verdict, evaluate};
pub use generate::{Simulation, TruthRun, generate};
pub use scenario::{Faults, PRESETS, Scenario};

/// シナリオをシード `seed` で生成し、計測ロジックで計算して、正解と照らし合わせる。
pub fn simulate(scenario: &Scenario, seed: u64) -> (Simulation, lapper_core::Outcome, Evaluation) {
    check(generate(scenario, seed))
}

/// 生成したセッションを計測ロジックで計算して、正解と照らし合わせる。
///
/// 計測ロジックの設定だけを変えて試すときは、`sim.config` を書き換えてから呼ぶ。
pub fn check(sim: Simulation) -> (Simulation, lapper_core::Outcome, Evaluation) {
    let outcome = lapper_core::compute(&sim.config, &sim.log, Some(sim.end));
    let evaluation = evaluate(&sim, &outcome);
    (sim, outcome, evaluation)
}
