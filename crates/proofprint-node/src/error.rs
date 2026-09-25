//! One failure type for node operations and their HTTP rendering.

use axum::extract::multipart::{MultipartError, MultipartRejection};
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use proofprint_core::Error as LedgerError;
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum NodeError {
    #[error(transparent)]
    Ledger(#[from] LedgerError),
    #[error("signing is disabled: the node was started without a key")]
    SigningDisabled,
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    BadRequest(String),
    /// A request the framework refused before a handler ran, with its status.
    #[error("{1}")]
    Rejected(StatusCode, String),
    #[error("{0}")]
    Internal(String),
}

impl NodeError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::BadRequest(message.into())
    }

    pub fn internal(error: impl std::fmt::Display) -> Self {
        Self::Internal(error.to_string())
    }

    pub fn status(&self) -> StatusCode {
        match self {
            Self::Ledger(LedgerError::RecordNotFound(_) | LedgerError::BlobNotFound(_)) => {
                StatusCode::NOT_FOUND
            }
            Self::Ledger(LedgerError::Io(_)) | Self::Internal(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
            Self::Ledger(_) | Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::SigningDisabled => StatusCode::SERVICE_UNAVAILABLE,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Rejected(status, _) => *status,
        }
    }
}

impl From<JsonRejection> for NodeError {
    fn from(rejection: JsonRejection) -> Self {
        Self::Rejected(rejection.status(), rejection.body_text())
    }
}

impl From<QueryRejection> for NodeError {
    fn from(rejection: QueryRejection) -> Self {
        Self::Rejected(rejection.status(), rejection.body_text())
    }
}

impl From<MultipartRejection> for NodeError {
    fn from(rejection: MultipartRejection) -> Self {
        Self::Rejected(rejection.status(), rejection.body_text())
    }
}

impl From<MultipartError> for NodeError {
    fn from(error: MultipartError) -> Self {
        Self::Rejected(error.status(), error.body_text())
    }
}

impl IntoResponse for NodeError {
    fn into_response(self) -> Response {
        let status = self.status();
        let message = if status == StatusCode::INTERNAL_SERVER_ERROR {
            tracing::error!(error = %self, "node operation failed");
            "node operation failed; see the server log".to_owned()
        } else {
            self.to_string()
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}
