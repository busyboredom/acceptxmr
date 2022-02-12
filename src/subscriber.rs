//! Subscribers should be used to receive invoice updates.

use std::error::Error;
use std::fmt;
use std::sync::mpsc::{RecvTimeoutError, TryRecvError};
use std::time::{Duration, Instant};

use sled::Event;

use crate::{invoices_db::InvoiceStorageError, AcceptXmrError, Invoice};

/// A means of receiving updates on a given invoice. Subscribers are returned by
/// [`PaymentGateways`](crate::PaymentGateway) when subscribing to a invoice.
pub struct Subscriber(sled::Subscriber);

impl Subscriber {
    pub(crate) fn new(sled_subscriber: sled::Subscriber) -> Subscriber {
        Subscriber(sled_subscriber)
    }

    /// Attempts to wait for a invoice update from this subscriber.
    ///
    /// # Errors
    ///
    /// Returns an error if the channel is closed, or if there is an error deserializing the update.
    pub fn recv(&mut self) -> Result<Invoice, AcceptXmrError> {
        let maybe_event = self.0.next();
        match maybe_event {
            Some(Event::Insert { value, .. }) => {
                bincode::decode_from_slice(&value, bincode::config::standard())
                    .map_err(|e| AcceptXmrError::from(InvoiceStorageError::from(e)))
                    .map(|tup| tup.0)
            }
            Some(Event::Remove { .. }) => self.recv(),
            None => Err(AcceptXmrError::Subscriber(SubscriberError::Recv)),
        }
    }

    /// Attempts to wait for a invoice update from this subscriber without blocking. Returns
    /// immediately if no update is available.
    ///
    /// # Errors
    ///
    /// Returns an error if the channel is closed, if there is no update, or if there is an error
    /// deserializing the update.
    pub fn try_recv(&mut self) -> Result<Invoice, AcceptXmrError> {
        // TODO: This shouldn't be using a timeout, but I am unaware of a better way to do it
        // given the limited options made available by sled.
        match self.0.next_timeout(Duration::from_nanos(0)) {
            Ok(Event::Insert { value, .. }) => {
                bincode::decode_from_slice(&value, bincode::config::standard())
                    .map_err(|e| AcceptXmrError::from(InvoiceStorageError::from(e)))
                    .map(|tup| tup.0)
            }
            Ok(Event::Remove { .. }) => self.try_recv(),
            Err(RecvTimeoutError::Timeout) => Err(AcceptXmrError::from(SubscriberError::TryRecv(
                TryRecvError::Empty,
            ))),
            Err(RecvTimeoutError::Disconnected) => Err(AcceptXmrError::from(
                SubscriberError::TryRecv(TryRecvError::Disconnected),
            )),
        }
    }

    /// Attempts to wait for a invoice update from this subscriber, returning an error if no update
    /// arrives within the provided `Duration`.
    ///
    /// # Errors
    ///
    /// Returns an error if the channel is closed, if an update is not received in time, or if there
    /// is an error deserializing the update.
    pub fn recv_timeout(&mut self, timeout: Duration) -> Result<Invoice, AcceptXmrError> {
        let start = Instant::now();
        loop {
            let event_or_err = self.0.next_timeout(timeout - start.elapsed());
            match event_or_err {
                Ok(Event::Insert { value, .. }) => {
                    return bincode::decode_from_slice(&value, bincode::config::standard())
                        .map_err(|e| AcceptXmrError::from(InvoiceStorageError::from(e)))
                        .map(|tup| tup.0)
                }
                Ok(Event::Remove { .. }) => continue,
                Err(e) => return Err(AcceptXmrError::Subscriber(SubscriberError::RecvTimeout(e))),
            }
        }
    }
}

impl Iterator for Subscriber {
    type Item = Result<Invoice, AcceptXmrError>;

    fn next(&mut self) -> Option<Result<Invoice, AcceptXmrError>> {
        // TODO: This shouldn't be using a timeout, but I am unaware of a better way to do it
        // given the limited options made available by sled.
        match self.0.next_timeout(Duration::from_nanos(0)) {
            Ok(Event::Insert { value, .. }) => Some(
                bincode::decode_from_slice(&value, bincode::config::standard())
                    .map_err(|e| AcceptXmrError::from(InvoiceStorageError::from(e)))
                    .map(|tup| tup.0),
            ),
            _ => None,
        }
    }
}

/// An error occurring while receiving invoice updates.
#[derive(Debug)]
pub enum SubscriberError {
    /// Failed to retrieve update.
    Recv,
    /// Timed out before receiving update.
    RecvTimeout(RecvTimeoutError),
    /// Subscriber is either empty or disconnected.
    TryRecv(TryRecvError),
}

impl fmt::Display for SubscriberError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SubscriberError::Recv => write!(
                f,
                "subscriber cannot receive further updates, likely because the scanning thread has panicked"
            ),
            SubscriberError::RecvTimeout(e) => write!(
                f,
                "subscriber recv timeout: {}", e
            ),
            SubscriberError::TryRecv(e) => write!(
                f,
                "subscriber try recv failed: {}", e
            ),
        }
    }
}

impl Error for SubscriberError {}
