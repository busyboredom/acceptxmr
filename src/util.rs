use std::error::Error;
use std::fmt;

use crate::rpc::RpcError;
use crate::SubIndex;
use crate::{invoices_db::InvoiceStorageError, subscriber::SubscriberError};

/// Library's custom error type.
#[derive(Debug)]
pub enum AcceptXmrError {
    /// An error originating from a daemon RPC call.
    Rpc(RpcError),
    /// An error storing/retrieving [`Invoice`](crate::Invoice)s.
    InvoiceStorage(InvoiceStorageError),
    /// [`Subscriber`](crate::Subscriber) failed to retrieve update.
    Subscriber(SubscriberError),
    /// Failure to unblind the amount of an owned output.
    Unblind(SubIndex),
}

impl From<RpcError> for AcceptXmrError {
    fn from(e: RpcError) -> Self {
        Self::Rpc(e)
    }
}

impl From<InvoiceStorageError> for AcceptXmrError {
    fn from(e: InvoiceStorageError) -> Self {
        Self::InvoiceStorage(e)
    }
}

impl From<SubscriberError> for AcceptXmrError {
    fn from(e: SubscriberError) -> Self {
        Self::Subscriber(e)
    }
}

impl fmt::Display for AcceptXmrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AcceptXmrError::Rpc(e) => {
                write!(f, "RPC error: {}", e)
            }
            AcceptXmrError::InvoiceStorage(e) => {
                write!(f, "invoice storage error: {}", e)
            }
            AcceptXmrError::Subscriber(e) => {
                write!(f, "subscriber failed to receive update: {}", e)
            }
            AcceptXmrError::Unblind(index) => write!(
                f,
                "unable to unblind amount of owned output sent to subaddress index {}",
                index
            ),
        }
    }
}

impl Error for AcceptXmrError {}
