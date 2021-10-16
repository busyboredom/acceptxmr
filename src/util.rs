use std::error::Error;
use std::fmt;

use crate::payments_db::PaymentStorageError;
use crate::rcp::RpcError;

#[derive(Debug)]
pub enum AcceptXmrError {
    Rpc(RpcError),
    PaymentStorage(PaymentStorageError),
    SubscriberRecv,
}

impl From<RpcError> for AcceptXmrError {
    fn from(e: RpcError) -> Self {
        Self::Rpc(e)
    }
}

impl From<PaymentStorageError> for AcceptXmrError {
    fn from(e: PaymentStorageError) -> Self {
        Self::PaymentStorage(e)
    }
}

impl fmt::Display for AcceptXmrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AcceptXmrError::Rpc(reqwest_error) => {
                write!(f, "RPC error: {}", reqwest_error)
            }
            AcceptXmrError::PaymentStorage(payment_storage_error) => {
                write!(f, "payment storage error: {}", payment_storage_error)
            }
            AcceptXmrError::SubscriberRecv => write!(
                f,
                "subscriber cannot receive further updates because the update source has shut down"
            ),
        }
    }
}

impl Error for AcceptXmrError {}
