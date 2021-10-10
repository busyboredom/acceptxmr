use std::error::Error;
use std::fmt;

use crate::payments_db::PaymentStorageError;

#[derive(Debug)]
pub enum AcceptXMRError {
    Rpc(reqwest::Error),
    PaymentStorage(PaymentStorageError),
    SubscriberRecv,
}

impl From<reqwest::Error> for AcceptXMRError {
    fn from(e: reqwest::Error) -> Self {
        Self::Rpc(e)
    }
}

impl From<PaymentStorageError> for AcceptXMRError {
    fn from(e: PaymentStorageError) -> Self {
        Self::PaymentStorage(e)
    }
}

impl fmt::Display for AcceptXMRError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AcceptXMRError::Rpc(reqwest_error) => {
                write!(f, "RPC request error: {}", reqwest_error)
            }
            AcceptXMRError::PaymentStorage(payment_storage_error) => {
                write!(f, "payment storage error: {}", payment_storage_error)
            }
            AcceptXMRError::SubscriberRecv => write!(
                f,
                "subscriber cannot receive further updates because the update source has shut down"
            ),
        }
    }
}

impl Error for AcceptXMRError {}
