//! 計測ボックスからの入力（TCP）。
//!
//! やり取りの形は [`lapper_protocol`]。届いた通過は、いちばん新しいセッションに追記する
//! （送り手は「ボックスの ID / 起動の ID」、番号はボックスの連番、通過記録の ID はサーバーが付ける）。
//! 保存できたら Ack を返す。セッションがないなどで保存できなければ返さないので、ボックスが後で送り直す。

use std::io;
use std::sync::Arc;

use lapper_core::{Event, GateId, Micros, Passage, PassageId, PassageSource};
use lapper_protocol::{BoxMessage, ServerMessage, decode, encode};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

use crate::hub::Hub;
use crate::types::{AppendEvents, NumberedEvent};

/// 計測ボックスからの接続を待ち受ける。接続ごとに別のタスクで処理する。
pub async fn serve(hub: Arc<Hub>, listener: TcpListener) {
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(e) => {
                eprintln!("box: cannot accept a connection: {e}");
                continue;
            }
        };
        let hub = hub.clone();
        tokio::spawn(async move {
            match connection(hub, stream).await {
                Ok(()) => println!("box {peer}: disconnected"),
                Err(e) => eprintln!("box {peer}: {e}"),
            }
        });
    }
}

async fn connection(hub: Arc<Hub>, stream: TcpStream) -> io::Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    // Hello で決まる送り手の ID
    let mut source: Option<String> = None;
    while let Some(line) = lines.next_line().await? {
        let message = match decode::<BoxMessage>(&line) {
            Ok(message) => message,
            Err(e) => {
                eprintln!("box: ignored a malformed message ({e}): {line}");
                continue;
            }
        };
        match message {
            BoxMessage::Hello { box_id, boot } => {
                println!("box {box_id}: connected (boot {boot})");
                source = Some(format!("{box_id}/{boot}"));
            }
            BoxMessage::Passage { seq, input, time } => {
                let Some(source) = &source else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "passage before hello",
                    ));
                };
                if record(&hub, source, seq, input, time) {
                    let ack = encode(&ServerMessage::Ack { seq });
                    writer.write_all(ack.as_bytes()).await?;
                }
            }
            BoxMessage::Clock { time } => {
                if let Some(session) = hub.latest_session() {
                    // セッションは消えないので、ここでは失敗しない
                    let _ = hub.set_clock(session, time);
                }
            }
        }
    }
    Ok(())
}

/// 通過をいちばん新しいセッションに追記する。保存できた（届き済みだった場合も含む）なら `true`。
fn record(hub: &Hub, source: &str, seq: u64, input: u16, time: Micros) -> bool {
    let Some(session) = hub.latest_session() else {
        eprintln!("box: no session to record passage {seq} in");
        return false;
    };
    let event = Event::Passage(Passage {
        // サーバーが付け直す（new_id）
        id: PassageId(0),
        gate: GateId(input),
        time,
        source: PassageSource::Sensor,
    });
    let append = AppendEvents {
        source: source.into(),
        events: vec![NumberedEvent {
            seq,
            event,
            at_now: false,
            new_id: true,
            age_ms: 0,
        }],
    };
    match hub.append(session, append) {
        Ok(_) => true,
        Err(e) => {
            eprintln!("box: cannot record passage {seq}: {e}");
            false
        }
    }
}
