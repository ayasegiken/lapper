//! 計測ボックスとサーバーのやり取り。
//!
//! 1 行に 1 つの JSON を送り合う（改行で区切る）。ボックスは接続したら最初に [`BoxMessage::Hello`] を送り、
//! その後、通過（[`BoxMessage::Passage`]）とコースの時計（[`BoxMessage::Clock`]、1 秒ごと）を送る。
//! サーバーは通過を保存したら [`ServerMessage::Ack`] を返す。ボックスは Ack が届くまで通過を溜めておき、送り直す。
//!
//! 入力端子の番号は、そのままゲートの ID として扱う。標準の割り当ては [`inputs`]。
//!
//! 詳しくは `docs/box-protocol.md`。

use lapper_core::Micros;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// 計測ボックスからサーバーへのメッセージ。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum BoxMessage {
    /// 接続したら最初に送る。
    Hello {
        /// ボックスの ID（機器ごとに決める）。
        box_id: String,
        /// 起動ごとに変わる ID。連番と時計は起動のたびに 0 から数え直すので、起動を区別するのに使う。
        boot: String,
    },
    /// 光電管の通過。
    Passage {
        /// 起動してからの連番（0 から）。サーバーはボックスと起動と連番の組で重複を除く。
        seq: u64,
        /// 入力端子の番号。今はそのままゲートの ID として扱う。
        input: u16,
        /// ボックスの時計で打刻した時刻（マイクロ秒）。
        time: Micros,
    },
    /// ボックスの時計の、今の時刻。通過がなくても 1 秒ごとに送る。
    Clock { time: Micros },
}

/// サーバーから計測ボックスへのメッセージ。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    /// 連番 `seq` の通過を保存した（届き済みだった場合も返す）。
    Ack { seq: u64 },
}

/// 計測ボックスの入力端子の標準の割り当て。
///
/// 0 = スタート、1 = 中間 1、2 = 中間 2、3 = ゴール。中間の数を変えても割り当ては変えず、使わない入力は空けておく。
/// スタートとゴールが同じ光電管なら、ゴールも 0 にする。
pub mod inputs {
    use lapper_core::{Course, GateId};

    pub const START: GateId = GateId(0);
    pub const FINISH: GateId = GateId(3);
    /// 中間計測地点の最大の数（入力 4 つのうち、スタートとゴール以外）。
    pub const MAX_INTERMEDIATES: usize = 2;

    /// 中間 `n` 番目（1 から）の入力。
    pub fn intermediate(n: usize) -> GateId {
        assert!((1..=MAX_INTERMEDIATES).contains(&n), "no intermediate {n}");
        GateId(n as u16)
    }

    /// 標準の割り当てでのコース。`same_gate` なら、スタートとゴールが同じ光電管。
    ///
    /// `intermediates` が [`MAX_INTERMEDIATES`] を超えると panic する。
    pub fn course(intermediates: usize, same_gate: bool) -> Course {
        let finish = if same_gate { START } else { FINISH };
        let gates = std::iter::once(START)
            .chain((1..=intermediates).map(intermediate))
            .chain(std::iter::once(finish))
            .collect();
        Course::new(gates).expect("start and finish make a valid course")
    }
}

/// メッセージを 1 行の JSON にする（末尾に改行を付ける）。
pub fn encode<T: Serialize>(message: &T) -> String {
    // 文字列と数値だけの列挙型なので、JSON への変換は失敗しない
    let mut line = serde_json::to_string(message).expect("message is serializable");
    line.push('\n');
    line
}

/// 1 行の JSON をメッセージにする（末尾の改行はあってもなくてもよい）。
pub fn decode<T: DeserializeOwned>(line: &str) -> Result<T, serde_json::Error> {
    serde_json::from_str(line.trim_end())
}

#[cfg(test)]
mod tests {
    use lapper_core::{Course, GateId};

    use super::*;

    #[test]
    fn standard_courses_keep_the_input_wiring() {
        let course =
            |gates: &[u16]| Course::new(gates.iter().map(|&g| GateId(g)).collect()).unwrap();
        assert_eq!(inputs::course(0, false), course(&[0, 3]));
        assert_eq!(inputs::course(1, false), course(&[0, 1, 3]));
        assert_eq!(inputs::course(2, false), course(&[0, 1, 2, 3]));
        assert_eq!(inputs::course(0, true), course(&[0, 0]));
        assert_eq!(inputs::course(1, true), course(&[0, 1, 0]));
    }

    #[test]
    fn messages_are_json_lines_tagged_with_type() {
        let passage = BoxMessage::Passage {
            seq: 3,
            input: 1,
            time: 12_345_678,
        };
        let line = encode(&passage);
        assert_eq!(
            line,
            "{\"type\":\"Passage\",\"seq\":3,\"input\":1,\"time\":12345678}\n"
        );
        assert_eq!(decode::<BoxMessage>(&line).unwrap(), passage);

        let ack = encode(&ServerMessage::Ack { seq: 3 });
        assert_eq!(ack, "{\"type\":\"Ack\",\"seq\":3}\n");

        let hello: BoxMessage =
            decode(r#"{"type":"Hello","box_id":"box-1","boot":"a1b2"}"#).unwrap();
        assert_eq!(
            hello,
            BoxMessage::Hello {
                box_id: "box-1".into(),
                boot: "a1b2".into()
            }
        );
        assert!(decode::<BoxMessage>(r#"{"type":"Unknown"}"#).is_err());
    }
}
