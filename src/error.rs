use std::fmt;

use crate::payments_db::PaymentStorageErrorKind;

#[derive(Debug)]
pub enum Error {
    RpcError(reqwest::Error),
    PaymentStorageError(PaymentStorageErrorKind),
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        Self::RpcError(e)
    }
}

impl From<PaymentStorageErrorKind> for Error {
    fn from(e: PaymentStorageErrorKind) -> Self {
        Self::PaymentStorageError(e)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::RpcError(reqwest_error) => write!(f, "RPC request error: {}", reqwest_error),
            Error::PaymentStorageError(payment_storage_error) => {
                write!(f, "Payment storage error: {}", payment_storage_error)
            }
        }
    }
}
