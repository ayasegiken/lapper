//! HTTP の API と、画面のファイルの配信。
//!
//! | メソッド | パス | 内容 |
//! |---|---|---|
//! | GET | `/api/sessions` | セッションの一覧（新しい順） |
//! | POST | `/api/sessions` | セッションを作る（[`NewSession`]） |
//! | GET | `/api/sessions/{id}` | セッションの情報 |
//! | POST | `/api/sessions/{id}/events` | イベントを追記する（[`AppendEvents`]）。届き済みの番号のイベントは無視する |
//! | POST | `/api/sessions/{id}/clock` | コースの時計の時刻を知らせる（[`ClockSync`]） |
//! | GET | `/api/sessions/{id}/snapshot` | 最新の計算結果（[`Snapshot`]） |
//! | GET | `/api/sessions/{id}/stream` | 計算結果の SSE。接続時と、結果が変わるたびに `snapshot` イベントを送る |

use std::convert::Infallible;
use std::path::Path;
use std::sync::Arc;

use axum::extract::{Path as UrlPath, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::WatchStream;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeader;

use crate::error::{Error, Result};
use crate::hub::Hub;
use crate::types::{AppendEvents, Appended, ClockSync, NewSession, SessionInfo, Snapshot};

/// API のルーター。`web_dir` を渡すと、それ以外のパスでは画面のファイルを配る。
pub fn router(hub: Arc<Hub>, web_dir: Option<&Path>) -> Router {
    let api = Router::new()
        .route("/sessions", get(list_sessions).post(create_session))
        .route("/sessions/{id}", get(get_session))
        .route("/sessions/{id}/events", post(append_events))
        .route("/sessions/{id}/clock", post(set_clock))
        .route("/sessions/{id}/snapshot", get(get_snapshot))
        .route("/sessions/{id}/stream", get(stream));
    let app = Router::new().nest("/api", api).with_state(hub);
    match web_dir {
        Some(dir) => {
            let files = ServeDir::new(dir).fallback(ServeFile::new(dir.join("index.html")));
            // サーバーを更新したときに古い画面が残らないよう、ブラウザーに毎回確認させる（変わっていなければ 304）
            let files = SetResponseHeader::overriding(
                files,
                header::CACHE_CONTROL,
                HeaderValue::from_static("no-cache"),
            );
            app.fallback_service(files)
        }
        None => app,
    }
}

async fn list_sessions(State(hub): State<Arc<Hub>>) -> Json<Vec<SessionInfo>> {
    Json(hub.sessions())
}

async fn create_session(
    State(hub): State<Arc<Hub>>,
    Json(new): Json<NewSession>,
) -> Result<(StatusCode, Json<SessionInfo>)> {
    Ok((StatusCode::CREATED, Json(hub.create_session(new)?)))
}

async fn get_session(
    State(hub): State<Arc<Hub>>,
    UrlPath(id): UrlPath<i64>,
) -> Result<Json<SessionInfo>> {
    Ok(Json(hub.session(id)?))
}

async fn append_events(
    State(hub): State<Arc<Hub>>,
    UrlPath(id): UrlPath<i64>,
    Json(body): Json<AppendEvents>,
) -> Result<Json<Appended>> {
    Ok(Json(hub.append(id, body)?))
}

async fn set_clock(
    State(hub): State<Arc<Hub>>,
    UrlPath(id): UrlPath<i64>,
    Json(body): Json<ClockSync>,
) -> Result<StatusCode> {
    hub.set_clock(id, body.time)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_snapshot(
    State(hub): State<Arc<Hub>>,
    UrlPath(id): UrlPath<i64>,
) -> Result<Json<Arc<Snapshot>>> {
    Ok(Json(hub.snapshot(id)?))
}

async fn stream(
    State(hub): State<Arc<Hub>>,
    UrlPath(id): UrlPath<i64>,
) -> Result<Sse<impl tokio_stream::Stream<Item = std::result::Result<SseEvent, Infallible>>>> {
    let snapshots = WatchStream::new(hub.subscribe(id)?);
    let events = snapshots.map(|snapshot| {
        // Snapshot の JSON への変換は失敗しない
        let data = serde_json::to_string(&*snapshot).expect("snapshot is serializable");
        Ok(SseEvent::default().event("snapshot").data(data))
    });
    Ok(Sse::new(events).keep_alive(KeepAlive::default()))
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = match self {
            Error::NotFound => StatusCode::NOT_FOUND,
            Error::BadRequest(_) => StatusCode::BAD_REQUEST,
            Error::DbVersion(_) | Error::Db(_) | Error::Json(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        (status, self.to_string()).into_response()
    }
}
