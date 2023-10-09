//! The AcceptXMR-Server HTTP API.

mod external;
pub(crate) mod internal;

use acceptxmr::{AcceptXmrError, Invoice, InvoiceId};
use axum::response::{IntoResponse, Response};
pub(crate) use external::external;
use hyper::{
    http::{uri::InvalidUri, Error as HttpError},
    StatusCode,
};
pub(crate) use internal::internal;
use log::error;
use serde::{Deserialize, Serialize};
use serde_json::Error as JsonError;
use thiserror::Error;
use utoipa::{IntoParams, ToSchema};

#[derive(Deserialize, ToSchema, IntoParams, Serialize)]
struct InvoiceIdPayload {
    #[schema(example = 4353165978)]
    invoice_id: InvoiceId,
}

impl From<InvoiceId> for InvoiceIdPayload {
    fn from(value: InvoiceId) -> Self {
        Self { invoice_id: value }
    }
}

/// An invoice update meant to be sent over the HTTP API.
#[derive(Serialize, Deserialize)]
pub struct InvoiceUpdate {
    /// The ID of the invoice.
    pub id: InvoiceId,
    /// The XMR address.
    pub address: String,
    /// The payment URI.
    pub uri: String,
    /// The amount requested in piconeros.
    pub amount_requested: u64,
    /// The amount paid in piconeros.
    pub amount_paid: u64,
    /// The number of confirmations required.
    pub confirmations_required: u64,
    /// The number of confirmations received, or `None` if the invoice is fully
    /// paid yet.
    pub confirmations: Option<u64>,
    /// The number of blocks until invoice exiration.
    pub expiration_in: u64,
    /// The current block height of the payment gateway.
    pub current_height: u64,
    /// The message associated with the invoice.
    pub message: String,
    /// The callback associated with the invoice.
    pub callback: Option<String>,
}

impl From<Invoice> for InvoiceUpdate {
    fn from(value: Invoice) -> Self {
        let InvoiceDescription { message, callback } =
            InvoiceDescription::from_json_or_any(value.description().to_string());

        InvoiceUpdate {
            id: value.id(),
            address: value.address().to_string(),
            uri: value.uri(),
            amount_requested: value.amount_requested(),
            amount_paid: value.amount_paid(),
            confirmations_required: value.confirmations_required(),
            confirmations: value.confirmations(),
            expiration_in: value.expiration_in(),
            current_height: value.current_height(),
            message,
            callback,
        }
    }
}

#[derive(Deserialize, Serialize)]
pub(crate) struct InvoiceDescription {
    pub(crate) message: String,
    pub(crate) callback: Option<String>,
}

impl InvoiceDescription {
    /// Attempt to deserialize from json string. On failure, assume the
    /// description is a message only.
    pub(crate) fn from_json_or_any(value: String) -> Self {
        serde_json::from_str(&value).unwrap_or(InvoiceDescription {
            message: value,
            callback: None,
        })
    }
}

#[derive(Error, Debug)]
enum ApiError {
    #[error(transparent)]
    AcceptXmr(#[from] AcceptXmrError),
    #[error("failed to serialize invoice description: {0}")]
    DescriptionSerialization(JsonError),
    #[error("invalid callback URI: {0}")]
    InvalidCallback(InvalidUri),
    #[error("failed to build HTTP response: {0}")]
    InvalidResponse(#[from] HttpError),
    #[error("invoice with ID {0} not found")]
    InvoiceNotFound(InvoiceId),
}

impl ApiError {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::AcceptXmr(_) | Self::InvalidResponse(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::DescriptionSerialization(_) | Self::InvalidCallback(_) => StatusCode::BAD_REQUEST,
            Self::InvoiceNotFound(_) => StatusCode::NOT_FOUND,
        }
    }

    fn message(&self) -> &'static str {
        match self {
            Self::AcceptXmr(_) => "Internal payment gateway error",
            Self::DescriptionSerialization(_) => "Failed to serialize invoice description",
            Self::InvalidCallback(_) => "Callback is not a valid URI",
            Self::InvalidResponse(_) => "Failed to build HTTP response",
            Self::InvoiceNotFound(_) => "invoice not found",
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status_code(), self.message()).into_response()
    }
}
