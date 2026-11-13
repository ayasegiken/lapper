//! 計測ボックスからの入力を、実際に TCP でつないで確かめる。

use std::sync::Arc;
use std::time::Duration;

use lapper_core::*;
use lapper_protocol::{BoxMessage, ServerMessage, decode, encode};
use lapper_server::types::NewSession;
use lapper_server::{Hub, Store, link};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

async fn start() -> (Arc<Hub>, TcpStream) {
    let hub = Arc::new(Hub::open(Store::open_in_memory().unwrap()).unwrap());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(link::serve(hub.clone(), listener));
    (hub, TcpStream::connect(addr).await.unwrap())
}

fn session(hub: &Hub) -> i64 {
    let course = Course::new(vec![GateId(0), GateId(1)]).unwrap();
    hub.create_session(NewSession {
        name: "box".into(),
        config: Config::new(course),
        gate_names: None,
    })
    .unwrap()
    .id
}

#[tokio::test]
async fn passages_are_recorded_once_and_acknowledged() {
    let (hub, stream) = start().await;
    let id = session(&hub);
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    let mut send = async |message: BoxMessage| {
        writer.write_all(encode(&message).as_bytes()).await.unwrap();
    };

    send(BoxMessage::Hello {
        box_id: "box-1".into(),
        boot: "b1".into(),
    })
    .await;
    let start = BoxMessage::Passage {
        seq: 0,
        input: 0,
        time: 10 * SECOND,
    };
    send(start.clone()).await;
    // Ack が届かなかったことにして送り直す
    send(start).await;
    send(BoxMessage::Passage {
        seq: 1,
        input: 1,
        time: 70 * SECOND,
    })
    .await;
    send(BoxMessage::Clock { time: 71 * SECOND }).await;

    for seq in [0, 0, 1] {
        let line = lines.next_line().await.unwrap().unwrap();
        assert_eq!(
            decode::<ServerMessage>(&line).unwrap(),
            ServerMessage::Ack { seq }
        );
    }

    let snapshot = hub.snapshot(id).unwrap();
    assert_eq!(snapshot.events, 2);
    let passages = &snapshot.outcome.passages;
    assert_eq!(passages[0].id, PassageId(1));
    assert_eq!(passages[1].id, PassageId(2));
    assert_eq!(passages[1].gate, GateId(1));
    assert_eq!(
        snapshot.outcome.runs[0].result,
        RunResult::Time(60 * SECOND)
    );
}

#[tokio::test]
async fn passages_are_not_acknowledged_without_a_session() {
    let (hub, stream) = start().await;
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    let hello = BoxMessage::Hello {
        box_id: "box-1".into(),
        boot: "b1".into(),
    };
    let passage = BoxMessage::Passage {
        seq: 0,
        input: 0,
        time: SECOND,
    };
    writer.write_all(encode(&hello).as_bytes()).await.unwrap();
    writer.write_all(encode(&passage).as_bytes()).await.unwrap();
    // セッションがないので保存できず、Ack は返らない
    let waited = tokio::time::timeout(Duration::from_millis(200), lines.next_line()).await;
    assert!(waited.is_err(), "unexpected reply: {waited:?}");

    // セッションを作ってから送り直すと、保存されて Ack が返る
    let id = session(&hub);
    writer.write_all(encode(&passage).as_bytes()).await.unwrap();
    let line = lines.next_line().await.unwrap().unwrap();
    assert_eq!(
        decode::<ServerMessage>(&line).unwrap(),
        ServerMessage::Ack { seq: 0 }
    );
    assert_eq!(hub.snapshot(id).unwrap().events, 1);
}
