use std::fmt;

/// コースの時計（計測ボックスの時計）でのマイクロ秒。
///
/// 時刻と時間の長さの両方に使う。
pub type Micros = i64;

/// 1 秒あたりのマイクロ秒。
pub const SECOND: Micros = 1_000_000;

/// 1 ミリ秒あたりのマイクロ秒。
pub const MILLISECOND: Micros = 1_000;

/// 光電管（計測ボックスの入力チャンネル）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GateId(pub u16);

/// 通過記録の ID。計測ボックスの連番などから作る、ログ全体で一意な値。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PassageId(pub u64);

/// 車（ゼッケン）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CarId(pub u32);

/// 走行の ID。走行を始めたスタートの通過記録の ID をそのまま使う。
///
/// 割り当てを計算し直しても、同じスタートの通過から始まる走行は同じ ID になる。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RunId(pub PassageId);

impl fmt::Display for GateId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "G{}", self.0)
    }
}

impl fmt::Display for PassageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "P{}", self.0)
    }
}

impl fmt::Display for CarId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "R{}", (self.0).0)
    }
}

/// 時間を `秒.ミリ秒`（例: `63.042`）の形にする。負の値にも対応する。
pub fn format_time(t: Micros) -> String {
    let sign = if t < 0 { "-" } else { "" };
    let ms = t.unsigned_abs() / 1_000;
    format!("{sign}{}.{:03}", ms / 1_000, ms % 1_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_time_in_milliseconds() {
        assert_eq!(format_time(63_042_999), "63.042");
        assert_eq!(format_time(0), "0.000");
        assert_eq!(format_time(-1_500_000), "-1.500");
    }
}
