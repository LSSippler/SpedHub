use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

/// Fehler, der als JSON `{ "error": "..." }` mit passendem Statuscode rausgeht.
#[derive(Debug)]
pub struct ApiError(pub StatusCode, pub String);

pub type ApiResult<T> = Result<T, ApiError>;

impl ApiError {
    pub fn bad(msg: &str) -> Self {
        Self(StatusCode::BAD_REQUEST, msg.into())
    }
    pub fn unauthorized() -> Self {
        Self(StatusCode::UNAUTHORIZED, "Nicht angemeldet".into())
    }
    pub fn forbidden(msg: &str) -> Self {
        Self(StatusCode::FORBIDDEN, msg.into())
    }
    pub fn not_found(msg: &str) -> Self {
        Self(StatusCode::NOT_FOUND, msg.into())
    }
    pub fn conflict(msg: &str) -> Self {
        Self(StatusCode::CONFLICT, msg.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(e: rusqlite::Error) -> Self {
        eprintln!("Datenbankfehler: {e}");
        Self(StatusCode::INTERNAL_SERVER_ERROR, "Datenbankfehler".into())
    }
}
