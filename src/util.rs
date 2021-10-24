use std::error::Error;
use std::fmt;

use crate::payments_db::PaymentStorageError;
use crate::rpc::RpcError;
use crate::SubIndex;

/// Library's custom error type.
#[derive(Debug)]
pub enum AcceptXmrError {
    /// An error originating from a daemon RPC call.
    Rpc(RpcError),
    /// An error storing/retrieving [`Payment`](crate::Payment)s.
    PaymentStorage(PaymentStorageError),
    /// [`Subscriber`](crate::Subscriber) failed to retrieve update.
    SubscriberRecv,
    /// [`Subscriber`](crate::Subscriber) timed out before receiving update.
    SubscriberRecvTimeout,
    /// Failure to unblind the amount of an owned output.
    Unblind(SubIndex),
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
            AcceptXmrError::Rpc(e) => {
                write!(f, "RPC error: {}", e)
            }
            AcceptXmrError::PaymentStorage(e) => {
                write!(f, "payment storage error: {}", e)
            }
            AcceptXmrError::SubscriberRecv => write!(
                f,
                "subscriber cannot receive further updates, likely because the scanning thread has panicked"
            ),
            AcceptXmrError::SubscriberRecvTimeout => write!(
                f,
                "subscriber recv timeout"
            ),
            AcceptXmrError::Unblind(index) => write!(
                f,
                "unable to unblind amount of owned output sent to subaddress index {}", index
            ),
        }
    }
}

impl Error for AcceptXmrError {}
