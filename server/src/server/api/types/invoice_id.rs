//! Invoice ID types.

use std::fmt::Display;

use acceptxmr::InvoiceId;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, DecodeError, Engine};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::{IntoParams, ToSchema};

/// An un-padded URL-safe base64 encoded invoice ID. This is done because
/// JS does not support 128 bit integers without `BigInt`, which ultimately
/// gets encoded as a String anyway.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct Base64InvoiceId(String);

impl Display for Base64InvoiceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl From<InvoiceId> for Base64InvoiceId {
    fn from(value: InvoiceId) -> Self {
        Self(URL_SAFE_NO_PAD.encode(u128::from(value).to_be_bytes()))
    }
}

impl TryFrom<Base64InvoiceId> for InvoiceId {
    type Error = InvoiceIdParseError;

    fn try_from(value: Base64InvoiceId) -> Result<Self, Self::Error> {
        invoice_id_from_str(&value.to_string())
    }
}

impl TryFrom<&str> for Base64InvoiceId {
    type Error = InvoiceIdParseError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let invoice_id = invoice_id_from_str(value)?;
        Ok(invoice_id.into())
    }
}

fn invoice_id_from_str(value: &str) -> Result<InvoiceId, InvoiceIdParseError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(InvoiceIdParseError::InvalidCharacters)?;
    let mut byte_array = [0u8; 16];

    // `copy_from_slice` will panic if the source is not large enough, so check
    // length first.
    match bytes.len() {
        len if len > 16 => return Err(InvoiceIdParseError::TooLong(bytes.len())),
        len if len < 16 => return Err(InvoiceIdParseError::TooShort(bytes.len())),
        _ => {}
    }

    byte_array.copy_from_slice(&bytes[..16]);
    let invoice_id = InvoiceId::from(u128::from_be_bytes(byte_array));

    Ok(invoice_id)
}

#[derive(Deserialize, Clone, ToSchema, IntoParams, Serialize)]
pub(crate) struct InvoiceIdPayload {
    #[schema(example = "AAAAAAAAAEkAAAAAACXOWQ")]
    pub(crate) invoice_id: Base64InvoiceId,
}

impl From<InvoiceId> for InvoiceIdPayload {
    fn from(value: InvoiceId) -> Self {
        Self {
            invoice_id: Base64InvoiceId::from(value),
        }
    }
}

impl TryFrom<InvoiceIdPayload> for InvoiceId {
    type Error = InvoiceIdParseError;

    fn try_from(value: InvoiceIdPayload) -> Result<Self, Self::Error> {
        value.invoice_id.try_into()
    }
}

#[derive(Deserialize, Clone, ToSchema, IntoParams, Serialize)]
#[into_params(parameter_in = Query)]
pub(crate) struct InvoiceIdQuery {
    #[schema(example = "AAAAAAAAAEkAAAAAACXOWQ")]
    id: Base64InvoiceId,
}

impl From<InvoiceId> for InvoiceIdQuery {
    fn from(value: InvoiceId) -> Self {
        Self {
            id: Base64InvoiceId::from(value),
        }
    }
}

impl TryFrom<InvoiceIdQuery> for InvoiceId {
    type Error = InvoiceIdParseError;

    fn try_from(value: InvoiceIdQuery) -> Result<Self, Self::Error> {
        value.id.try_into()
    }
}

/// An error parsing an Invoice ID.
#[derive(Error, Debug)]
pub enum InvoiceIdParseError {
    /// Invoice ID was too long.
    #[error("Base64 invoice ID was too long. Got {0} bytes, expected 16 bytes")]
    TooLong(usize),
    /// Invoice ID was too short.
    #[error("Base64 invoice ID was too short. Got {0} bytes, expected 16 bytes")]
    TooShort(usize),
    /// Invalid characters in invoice ID.
    #[error("Base64 invoice ID was not valid base64: {0}")]
    InvalidCharacters(DecodeError),
}

#[cfg(test)]
mod test {
    use acceptxmr::{InvoiceId, SubIndex};
    use test_case::test_case;

    use super::Base64InvoiceId;

    #[test_case(InvoiceId::new(SubIndex::new(0,0),0) => "AAAAAAAAAAAAAAAAAAAAAA")]
    #[test_case(InvoiceId::new(SubIndex::new(u32::MAX,0),0) => "_____wAAAAAAAAAAAAAAAA")]
    #[test_case(InvoiceId::new(SubIndex::new(0,u32::MAX),0) => "AAAAAP____8AAAAAAAAAAA")]
    #[test_case(InvoiceId::new(SubIndex::new(0,0),u64::MAX) => "AAAAAAAAAAD__________w")]
    #[test_case(InvoiceId::new(SubIndex::new(u32::MAX,u32::MAX),u64::MAX) => "_____________________w")]
    fn base64_invoice_roundtrip(invoice_id: InvoiceId) -> String {
        let base64_invoice_id = Base64InvoiceId::from(invoice_id);
        assert_eq!(
            InvoiceId::try_from(base64_invoice_id.clone()).unwrap(),
            invoice_id
        );
        base64_invoice_id.to_string()
    }

    #[test_case("AAAAAAAAAAAAAAAAAAAAAA" => Ok(InvoiceId::new(SubIndex::new(0,0), 0)))]
    #[test_case("I am an invalid base64 invoice ID" => Err(()))]
    #[test_case("Invalid" => Err(()))]
    #[test_case("" => Err(()))]
    fn invoice_id_try_from_base64(base64_invoice_id: &'static str) -> Result<InvoiceId, ()> {
        InvoiceId::try_from(Base64InvoiceId::try_from(base64_invoice_id).map_err(|_| ())?)
            .map_err(|_| ())
    }
}
