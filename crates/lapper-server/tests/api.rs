//! HTTP の API を、実際にリクエストを送って確かめる。

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use lapper_core::*;
use lapper_server::types::{Appended, SessionInfo, Snapshot};
use lapper_server::{Hub, Store, api};
use serde_json::{Value, json};
use tower::ServiceExt;

fn app() -> axum::Router {
    let hub = Hub::open(Store::open_in_memory().unwrap()).unwrap();
    api::router(Arc::new(hub), None)
}

async fn send(
    app: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Vec<u8>) {
    let request = Request::builder().method(method).uri(uri);
    let request = match body {
        Some(body) => request
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string())),
        None => request.body(Body::empty()),
    }
    .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, bytes.to_vec())
}

fn session_body() -> Value {
    json!({
        "name": "practice",
        "config": serde_json::to_value(Config::new(Course::new(vec![GateId(0), GateId(1)]).unwrap())).unwrap(),
    })
}

#[tokio::test]
async fn create_session_append_events_and_read_the_result() {
    let app = app();
    let (status, body) = send(&app, "POST", "/api/sessions", Some(session_body())).await;
    assert_eq!(status, StatusCode::CREATED);
    let session: SessionInfo = serde_json::from_slice(&body).unwrap();
    assert_eq!(session.gate_names[&GateId(1)], "G");

    let events = json!({ "events": [
        { "type": "Enqueue", "car": 7, "at": 0 },
        { "type": "Passage", "id": 1, "gate": 0, "time": 10_000_000, "source": "Sensor" },
        { "type": "Passage", "id": 2, "gate": 1, "time": 72_345_000, "source": "Sensor" },
    ]});
    let uri = format!("/api/sessions/{}/events", session.id);
    let (status, body) = send(&app, "POST", &uri, Some(events)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(serde_json::from_slice::<Appended>(&body).unwrap().events, 3);

    let uri = format!("/api/sessions/{}/snapshot", session.id);
    let (status, body) = send(&app, "GET", &uri, None).await;
    assert_eq!(status, StatusCode::OK);
    let snapshot: Snapshot = serde_json::from_slice(&body).unwrap();
    assert_eq!(snapshot.outcome.runs[0].car, Some(CarId(7)));
    assert_eq!(snapshot.outcome.runs[0].result, RunResult::Time(62_345_000));

    let (_, body) = send(&app, "GET", "/api/sessions", None).await;
    let sessions: Vec<SessionInfo> = serde_json::from_slice(&body).unwrap();
    assert_eq!(sessions, vec![session]);
}

#[tokio::test]
async fn errors_have_matching_status_codes() {
    let app = app();
    let (status, _) = send(&app, "GET", "/api/sessions/42/snapshot", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let mut body = session_body();
    body["config"]["course"] = json!([0]);
    let (status, _) = send(&app, "POST", "/api/sessions", Some(body)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    let mut body = session_body();
    body["name"] = json!(" ");
    let (status, _) = send(&app, "POST", "/api/sessions", Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn stream_sends_the_current_snapshot_first() {
    let app = app();
    let (_, body) = send(&app, "POST", "/api/sessions", Some(session_body())).await;
    let session: SessionInfo = serde_json::from_slice(&body).unwrap();

    let request = Request::get(format!("/api/sessions/{}/stream", session.id))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();
    let frame = body.frame().await.unwrap().unwrap().into_data().unwrap();
    let text = String::from_utf8(frame.to_vec()).unwrap();
    assert!(text.starts_with("event: snapshot\ndata: "), "{text}");
}
