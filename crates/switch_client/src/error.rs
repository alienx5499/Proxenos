//! Typed client errors.

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("network: {0}")]
    Http(#[from] reqwest::Error),
    #[error("switch answered {status}: {body}")]
    Status { status: u16, body: String },
    #[error("bad json from switch: {0}")]
    Json(#[from] serde_json::Error),
    #[error("protocol: {0}")]
    Protocol(String),
    /// Another process attached to this connection id; reopening would fight it.
    #[error("connection taken over by another stream")]
    TakenOver,
}

impl Error {
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Status { status, .. } => Some(*status),
            _ => None,
        }
    }
}
