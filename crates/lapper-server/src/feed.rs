//! シミュレーターで生成したセッションを、実際の時間の流れで流し込む。
//!
//! 計測ボックスがない段階で、画面の動きを確かめるための入力元。
//! イベントは届いた時刻（`Simulation::arrivals`）になったら追記し、コースの時計も定期的に知らせる。

use std::sync::Arc;
use std::time::{Duration, Instant};

use lapper_core::{Micros, SECOND};
use lapper_sim::Simulation;

use crate::error::Result;
use crate::hub::Hub;
use crate::types::{AppendEvents, NumberedEvent};

/// コースの時計を知らせる間隔。
const TICK: Duration = Duration::from_millis(100);

/// 追記するときの送り手の ID。
const SOURCE: &str = "simulator";

/// `sim` を `speed` 倍の速さでセッション `session` に流し込む。セッションの終わりまで進んだら戻る。
pub async fn feed(hub: Arc<Hub>, session: i64, sim: Simulation, speed: f64) -> Result<()> {
    // 最初のイベントの時刻から始める（待ち時間なしで流れ始めるように）
    let origin = sim.arrivals.first().copied().unwrap_or(0);
    let started = Instant::now();
    let mut next = 0;
    let mut tick = tokio::time::interval(TICK);
    loop {
        tick.tick().await;
        let elapsed = started.elapsed().as_secs_f64() * speed;
        let now = (origin + (elapsed * SECOND as f64) as Micros).min(sim.end);
        let due = sim.arrivals[next..]
            .iter()
            .take_while(|&&at| at <= now)
            .count();
        if due > 0 {
            // 番号はログの中での位置
            let events = (next..next + due)
                .map(|i| NumberedEvent {
                    seq: i as u64,
                    event: sim.log[i].clone(),
                    at_now: false,
                    new_id: false,
                    age_ms: 0,
                })
                .collect();
            hub.append(
                session,
                AppendEvents {
                    source: SOURCE.into(),
                    events,
                },
            )?;
            next += due;
        }
        hub.set_clock(session, now)?;
        if now >= sim.end {
            return Ok(());
        }
    }
}
