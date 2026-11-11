//! lapper の Web サーバー。
//!
//! イベント（通過記録、出走待ちの列の操作、訂正）をセッションごとに SQLite へ追記し、
//! 追記のたびに [`lapper_core::compute`] で結果を計算し直して、ブラウザーに SSE で配る。
//!
//! - [`store`] は SQLite への保存と読み込み
//! - [`hub`] はメモリー上のセッションと、結果の計算・配信
//! - [`api`] は HTTP の API と、画面のファイルの配信
//! - [`feed`] はシミュレーターで生成したセッションを、実際の時間の流れで流し込む

pub mod api;
pub mod error;
pub mod feed;
pub mod hub;
pub mod store;
pub mod types;

pub use error::{Error, Result};
pub use hub::Hub;
pub use store::Store;
