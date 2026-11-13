//! 計測ボックスとサーバーのやり取り。
//!
//! 1 行に 1 つの JSON を送り合う（改行で区切る）。ボックスは接続したら最初に [`BoxMessage::Hello`] を送り、
//! その後、通過（[`BoxMessage::Passage`]）とコースの時計（[`BoxMessage::Clock`]、1 秒ごと）を送る。
//! サーバーは通過を保存したら [`ServerMessage::Ack`] を返す。ボックスは Ack が届くまで通過を溜めておき、送り直す。
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
    use super::*;

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
