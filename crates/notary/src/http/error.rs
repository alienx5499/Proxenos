//! AppError to HTTP status; internals are logged, never sent.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use proxenos_app::AppError;
use proxenos_app::ports::RepoError;
use proxenos_domain::error::DomainError;
use serde_json::json;

#[derive(Debug)]
pub struct ApiError(pub StatusCode, pub String);

impl ApiError {
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self(StatusCode::BAD_REQUEST, msg.into())
    }

    pub fn unauthorized() -> Self {
        Self(StatusCode::UNAUTHORIZED, "operator token required".into())
    }

    pub fn not_found(msg: impl Into<String>) -> Self {
        Self(StatusCode::NOT_FOUND, msg.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

impl From<JsonRejection> for ApiError {
    fn from(r: JsonRejection) -> Self {
        Self(r.status(), r.body_text())
    }
}

impl From<DomainError> for ApiError {
    fn from(e: DomainError) -> Self {
        AppError::from(e).into()
    }
}

impl From<AppError> for ApiError {
    fn from(e: AppError) -> Self {
        use StatusCode as S;
        let status = match &e {
            AppError::DealNotFound(_) | AppError::UnknownSealToken => S::NOT_FOUND,
            AppError::DealExists(_)
            | AppError::RoomInUse(_)
            | AppError::Repo(RepoError::Conflict) => S::CONFLICT,
            AppError::Domain(d) => domain_status(d),
            AppError::WrongSide | AppError::AdmitNeedsTwoEach | AppError::ExcludedNegotiator => {
                S::UNPROCESSABLE_ENTITY
            }
            AppError::Switch(_) => {
                eprintln!("http: {e}");
                return Self(S::BAD_GATEWAY, "switch call failed".into());
            }
            AppError::Repo(_) | AppError::Vault(_) | AppError::Internal(_) => {
                eprintln!("http: {e}");
                return Self(S::INTERNAL_SERVER_ERROR, "internal error".into());
            }
            _ => S::CONFLICT,
        };
        Self(status, e.to_string())
    }
}

/// Bad input is 422; a move the deal's state refuses is 409.
fn domain_status(e: &DomainError) -> StatusCode {
    use DomainError as D;
    match e {
        D::CurrencyMismatch
        | D::MissingLimit(_)
        | D::UnexpectedLimit(_)
        | D::WrongDirection(_)
        | D::WrongValueType(_)
        | D::EmptyRange(_)
        | D::EmptyId(_)
        | D::InvalidDealId
        | D::InvalidOfferId
        | D::InvalidHash
        | D::NoteTooLong
        | D::InvalidMaxRounds
        | D::WrongLimitSet
        | D::KindMismatch => StatusCode::UNPROCESSABLE_ENTITY,
        D::InvalidRecord(_) => StatusCode::INTERNAL_SERVER_ERROR,
        _ => StatusCode::CONFLICT,
    }
}
