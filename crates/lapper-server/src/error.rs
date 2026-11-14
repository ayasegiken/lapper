use std::fmt;

#[derive(Debug)]
pub enum Error {
    /// 指定されたセッションがない。
    NotFound,
    /// 入力の内容がおかしい。
    BadRequest(String),
    /// データベースの形式の版が、このサーバーと合わない。
    DbVersion(i64),
    Db(rusqlite::Error),
    Json(serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotFound => write!(f, "not found"),
            Error::BadRequest(message) => write!(f, "bad request: {message}"),
            Error::DbVersion(version) => write!(
                f,
                "unsupported database version {version} (expected {}); use a new database file",
                crate::store::VERSION
            ),
            Error::Db(e) => write!(f, "database error: {e}"),
            Error::Json(e) => write!(f, "json error: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Db(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Json(e)
    }
}
