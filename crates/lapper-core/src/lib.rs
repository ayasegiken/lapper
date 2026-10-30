//! lapper の計測ロジック。
//!
//! 光電管の生の通過記録と、運営や参加者からの報告・訂正を追記だけのログ（[`Event`] の並び）として持ち、
//! 「どの通過がどの車の走行か」をログ全体から毎回計算し直す（[`compute`]）。
//! 報告が遅れても、後から訂正しても、関係する割り当てが自動で正しく直る。
//!
//! ```
//! use lapper_core::*;
//!
//! let (s, g) = (GateId(0), GateId(1));
//! let config = Config::new(Course::new(vec![s, g]).unwrap());
//! let pass = |id, gate, time| {
//!     Event::Passage(Passage { id: PassageId(id), gate, time, source: PassageSource::Sensor })
//! };
//! let log = vec![
//!     Event::Enqueue { car: CarId(7), at: 0 },
//!     pass(1, s, 10 * SECOND),
//!     pass(2, g, 72 * SECOND + 345 * MILLISECOND),
//! ];
//! let outcome = compute(&config, &log, None);
//! let run = &outcome.runs[0];
//! assert_eq!(run.car, Some(CarId(7)));
//! assert_eq!(run.result, RunResult::Time(62_345_000));
//! ```

mod config;
mod course;
mod engine;
mod event;
mod outcome;
mod types;

pub use config::{Config, GateConfig};
pub use course::{Course, CourseError};
pub use engine::compute;
pub use event::{CarChoice, Event, Passage, PassageSource, Penalty};
pub use outcome::{
    CarSource, Disposition, OffCause, Outcome, PassageReport, Run, RunFlag, RunResult, RunStatus,
    Slot, Standing, Warning,
};
pub use types::{CarId, GateId, MILLISECOND, Micros, PassageId, RunId, SECOND, format_time};
