use std::fmt;

use crate::types::GateId;

/// コース。ゲートの通過順の並びとして定義する。
///
/// 並びの位置を「ポジション」と呼ぶ。ポジション 0 がスタート、最後がゴールで、その間が中間計時。
/// 例:
///
/// - `[S, I1, I2, G]`: スタート、中間 2 つ、ゴール
/// - `[SG, SG]`: スタートとゴールが同じ光電管
/// - `[S, A, B, A, G]`: 同じゲート `A` を 2 回通る
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Course {
    gates: Vec<GateId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CourseError {
    /// スタートとゴールの 2 つ以上のポジションが必要。
    TooShort,
}

impl fmt::Display for CourseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CourseError::TooShort => write!(f, "a course needs at least a start and a finish"),
        }
    }
}

impl std::error::Error for CourseError {}

impl Course {
    pub fn new(gates: Vec<GateId>) -> Result<Course, CourseError> {
        if gates.len() < 2 {
            return Err(CourseError::TooShort);
        }
        Ok(Course { gates })
    }

    /// ポジションの数（スタートとゴールを含む）。
    pub fn len(&self) -> usize {
        self.gates.len()
    }

    /// 常に `false`（コースは 2 ポジション以上ある）。
    pub fn is_empty(&self) -> bool {
        false
    }

    /// 区間の数（ポジションの数 - 1）。区間 `i` はポジション `i` から `i + 1` まで。
    pub fn segments(&self) -> usize {
        self.gates.len() - 1
    }

    /// ゴールのポジション。
    pub fn finish(&self) -> usize {
        self.gates.len() - 1
    }

    pub fn gate_at(&self, position: usize) -> GateId {
        self.gates[position]
    }

    pub fn start_gate(&self) -> GateId {
        self.gates[0]
    }

    pub fn gates(&self) -> &[GateId] {
        &self.gates
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_course_without_finish() {
        assert_eq!(Course::new(vec![GateId(0)]), Err(CourseError::TooShort));
    }

    #[test]
    fn start_and_finish_can_share_a_gate() {
        let course = Course::new(vec![GateId(0), GateId(0)]).unwrap();
        assert_eq!(course.finish(), 1);
        assert_eq!(course.segments(), 1);
        assert_eq!(course.gate_at(course.finish()), course.start_gate());
    }
}
