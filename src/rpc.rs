//! JSON-RPC errors shared by the protocol boundary and feature handlers.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{borrow::Cow, fmt};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(from = "i64", into = "i64")]
pub enum ErrorCode {
    ParseError,
    InvalidRequest,
    MethodNotFound,
    InvalidParams,
    InternalError,
    RequestCancelled,
    ServerError(i64),
}

impl From<ErrorCode> for i64 {
    fn from(code: ErrorCode) -> Self {
        match code {
            ErrorCode::ParseError => -32700,
            ErrorCode::InvalidRequest => -32600,
            ErrorCode::MethodNotFound => -32601,
            ErrorCode::InvalidParams => -32602,
            ErrorCode::InternalError => -32603,
            ErrorCode::RequestCancelled => -32800,
            ErrorCode::ServerError(code) => code,
        }
    }
}

impl From<i64> for ErrorCode {
    fn from(code: i64) -> Self {
        match code {
            -32700 => Self::ParseError,
            -32600 => Self::InvalidRequest,
            -32601 => Self::MethodNotFound,
            -32602 => Self::InvalidParams,
            -32603 => Self::InternalError,
            -32800 => Self::RequestCancelled,
            code => Self::ServerError(code),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Error {
    pub code: ErrorCode,
    pub message: Cow<'static, str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl Error {
    pub fn invalid_params(message: impl Into<Cow<'static, str>>) -> Self {
        Self {
            code: ErrorCode::InvalidParams,
            message: message.into(),
            data: None,
        }
    }

    pub const fn invalid_request() -> Self {
        Self {
            code: ErrorCode::InvalidRequest,
            message: Cow::Borrowed("Invalid request"),
            data: None,
        }
    }

    pub const fn request_cancelled() -> Self {
        Self {
            code: ErrorCode::RequestCancelled,
            message: Cow::Borrowed("Request cancelled"),
            data: None,
        }
    }

    pub const fn internal_error() -> Self {
        Self {
            code: ErrorCode::InternalError,
            message: Cow::Borrowed("Internal error"),
            data: None,
        }
    }

    pub const fn method_not_found() -> Self {
        Self {
            code: ErrorCode::MethodNotFound,
            message: Cow::Borrowed("Method not found"),
            data: None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.message, i64::from(self.code))
    }
}

impl std::error::Error for Error {}
