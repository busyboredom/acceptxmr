//! The AcceptXMR-Server HTTP API.

mod external;
mod internal;
mod templating;
pub mod types;

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
use types::invoice_id::{Base64InvoiceId, InvoiceIdParseError};
use utoipa::ToSchema;

/// An invoice update meant to be sent over the HTTP API.
#[derive(Serialize, Deserialize, ToSchema)]
pub struct InvoiceUpdate {
    /// The un-padded URL-safe base64 encoded ID of the invoice.
    pub id: Base64InvoiceId,
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
    /// The number of confirmations received, or `None` if the invoice is not
    /// fully paid yet.
    pub confirmations: Option<u64>,
    /// The number of blocks until invoice exiration.
    pub expiration_in: u64,
    /// The current block height of the payment gateway.
    pub current_height: u64,
    /// The order associated with the invoice.
    pub order: String,
    /// The callback associated with the invoice.
    pub callback: Option<String>,
}

impl From<Invoice> for InvoiceUpdate {
    fn from(value: Invoice) -> Self {
        let InvoiceDescription { order, callback } =
            InvoiceDescription::from_json_or_any(value.description().to_string());

        InvoiceUpdate {
            id: value.id().into(),
            address: value.address().to_string(),
            uri: value.uri(),
            amount_requested: value.amount_requested(),
            amount_paid: value.amount_paid(),
            confirmations_required: value.confirmations_required(),
            confirmations: value.confirmations(),
            expiration_in: value.expiration_in(),
            current_height: value.current_height(),
            order,
            callback,
        }
    }
}

#[derive(Deserialize, Serialize)]
pub(crate) struct InvoiceDescription {
    pub(crate) order: String,
    pub(crate) callback: Option<String>,
}

impl InvoiceDescription {
    /// Attempt to deserialize from json string. On failure, assume the
    /// description is a order only.
    pub(crate) fn from_json_or_any(value: String) -> Self {
        serde_json::from_str(&value).unwrap_or(InvoiceDescription {
            order: value,
            callback: None,
        })
    }
}

/// An error that can be sent back over the API to the client.
#[derive(Error, Debug)]
pub enum ApiError {
    /// An error originating from the `AcceptXMR` library.
    #[error(transparent)]
    AcceptXmr(#[from] AcceptXmrError),
    /// Failed to serialize the invoice description.
    #[error("failed to serialize invoice description: {0}")]
    DescriptionSerialization(JsonError),
    /// Invalid callback URI.
    #[error("invalid callback URI: {0}")]
    InvalidCallback(InvalidUri),
    /// Failed to build HTTP response.
    #[error("failed to build HTTP response: {0}")]
    InvalidResponse(#[from] HttpError),
    /// Invoice not found.
    #[error("invoice with ID {0} not found")]
    InvoiceNotFound(InvoiceId),
    /// Invalid invoice ID.
    #[error("invoice ID could not be parsed: {0}")]
    InvalidInvoiceId(#[from] InvoiceIdParseError),
    /// Missing static resource.
    #[error("missing static resource: {0}")]
    MissingResource(#[from] std::io::Error),
    /// Templating error.
    #[error("failed to render template: {0}")]
    TemplatingError(#[from] tera::Error),
}

impl ApiError {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::AcceptXmr(_) | Self::InvalidResponse(_) | Self::TemplatingError(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
            Self::InvalidInvoiceId(_)
            | Self::DescriptionSerialization(_)
            | Self::InvalidCallback(_) => StatusCode::BAD_REQUEST,
            Self::MissingResource(_) | Self::InvoiceNotFound(_) => StatusCode::NOT_FOUND,
        }
    }

    fn message(&self) -> &'static str {
        match self {
            Self::AcceptXmr(_) => "Internal payment gateway error",
            Self::DescriptionSerialization(_) => "Failed to serialize invoice description",
            Self::InvalidCallback(_) => "Callback is not a valid URI",
            Self::InvalidResponse(_) => "Failed to build HTTP response",
            Self::InvoiceNotFound(_) => "Invoice not found",
            Self::InvalidInvoiceId(_) => "Invalid invoice ID",
            Self::MissingResource(_) => "Missing static resource",
            Self::TemplatingError(_) => "Failed to render template",
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status_code(), self.message()).into_response()
    }
}
