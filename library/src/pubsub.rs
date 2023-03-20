//! Subscribers should be used to receive invoice updates.

/// Max size of subscriber backlog.
const SUBSCRIPTION_BUFFER_LEN: usize = 2048;

use std::{
    collections::HashMap,
    fmt::Debug,
    future::Future,
    pin::Pin,
    sync::{Mutex, PoisonError},
    task::{Context, Poll},
    time::Duration,
};

use indexmap::IndexMap;
use log::warn;
use thiserror::Error;
use tokio::{
    sync::mpsc::{channel, error::TryRecvError, Receiver, Sender},
    time::error::Elapsed,
};

use crate::{Invoice, InvoiceId};

/// A means of receiving updates on a given invoice. Subscribers are returned by
/// [`PaymentGateways`](crate::PaymentGateway) when subscribing to a invoice.
pub struct Subscriber(Receiver<Invoice>);

impl Subscriber {
    pub(crate) fn new(receiver: Receiver<Invoice>) -> Subscriber {
        Subscriber(receiver)
    }

    /// Waits for a invoice update from this subscriber.
    ///
    /// Returns `None` if the channel is closed.
    pub async fn recv(&mut self) -> Option<Invoice> {
        self.0.recv().await
    }

    /// Blocks while waiting for a invoice update from this subscriber.
    ///
    /// Returns `None` if the channel is closed.
    ///
    /// # Panics
    ///
    /// This function panics if called within an asynchronous execution context.
    pub fn blocking_recv(&mut self) -> Option<Invoice> {
        self.0.blocking_recv()
    }

    /// Attempts to wait for a invoice update from this subscriber without
    /// blocking. Returns immediately if no update is available.
    ///
    /// # Errors
    ///
    /// Returns an error if the channel is closed or if there is no update.
    pub fn try_recv(&mut self) -> Result<Invoice, SubscriberError> {
        Ok(self.0.try_recv()?)
    }

    /// Attempts to wait for a invoice update from this subscriber, returning an
    /// error if no update arrives within the provided `Duration`. Returns
    /// `None` if the channel is closed.
    ///
    /// # Errors
    ///
    /// Returns an error if no update is received in time.
    pub async fn recv_timeout(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<Invoice>, SubscriberError> {
        Ok(tokio::time::timeout(timeout, self.0.recv()).await?)
    }
}

impl Future for Subscriber {
    type Output = Option<Invoice>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.0.poll_recv(cx)
    }
}

pub(crate) struct Publisher {
    invoice_subs: Mutex<HashMap<InvoiceId, IndexMap<SenderId, Sender<Invoice>>>>,
    global_subs: Mutex<IndexMap<SenderId, Sender<Invoice>>>,
}

impl Publisher {
    pub fn new() -> Publisher {
        Publisher {
            invoice_subs: Mutex::new(HashMap::new()),
            global_subs: Mutex::new(IndexMap::new()),
        }
    }

    pub fn subscribe(&self, invoice_id: InvoiceId) -> Option<Subscriber> {
        let (tx, rx) = channel(SUBSCRIPTION_BUFFER_LEN);
        let mut invoice_subs = self
            .invoice_subs
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        invoice_subs
            .get_mut(&invoice_id)?
            .insert(SenderId::new(), tx);
        Some(Subscriber::new(rx))
    }

    pub fn subscribe_all(&self) -> Subscriber {
        let (tx, rx) = channel(SUBSCRIPTION_BUFFER_LEN);
        let mut global_subs = self
            .global_subs
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        global_subs.insert(SenderId::new(), tx);
        Subscriber::new(rx)
    }

    pub fn insert_invoice(&self, invoice_id: InvoiceId) {
        let mut invoice_subs = self
            .invoice_subs
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        // Inserts the sub at the end of the indexmap.
        if invoice_subs.insert(invoice_id, IndexMap::new()).is_some() {
            warn!("Added invoice that is already being tracked; Subscribers overwritten.");
        }
    }

    pub fn remove_invoice(&self, invoice_id: InvoiceId) {
        let mut invoice_subs = self
            .invoice_subs
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        // Removes sub, moving the sub at the end of the indexmap into its place to
        // avoid shifting all proceeding elements.
        invoice_subs.remove(&invoice_id);
    }

    pub async fn send_updates(&self, invoice: &Invoice) {
        let mut index = 0;
        let mut sender_id;
        let mut closed = false;
        loop {
            match self.get_sender_by_index(Some(invoice.id()), index) {
                Some((id, sender)) => {
                    sender_id = id;
                    if sender.send(invoice.clone()).await.is_err() {
                        closed = true;
                    }
                }
                None => break,
            }
            if closed {
                self.remove_sender(Some(invoice.id()), sender_id);
            } else {
                index += 1;
            }
        }

        index = 0;
        closed = false;
        loop {
            match self.get_sender_by_index(None, index) {
                Some((id, sender)) => {
                    sender_id = id;
                    if sender.send(invoice.clone()).await.is_err() {
                        closed = true;
                    }
                }
                None => break,
            }
            if closed {
                self.remove_sender(None, sender_id);
            } else {
                index += 1;
            }
        }
    }

    fn get_sender_by_index(
        &self,
        invoice_id: Option<InvoiceId>,
        index: usize,
    ) -> Option<(SenderId, Sender<Invoice>)> {
        if let Some(id) = invoice_id {
            let mut invoice_subs = self
                .invoice_subs
                .lock()
                .unwrap_or_else(PoisonError::into_inner);

            invoice_subs
                .get_mut(&id)
                .and_then(|map| map.get_index(index))
                .map(|(id, s)| (*id, s.clone()))
        } else {
            let global_subs = self
                .global_subs
                .lock()
                .unwrap_or_else(PoisonError::into_inner);

            global_subs.get_index(index).map(|(id, s)| (*id, s.clone()))
        }
    }

    /// It's important that this function is only called within `send_updates`,
    /// because changing the order of senders could cause some [`Subscriber`]s
    /// to miss updates if done at the wrong time.
    fn remove_sender(&self, invoice_id: Option<InvoiceId>, sender_id: SenderId) {
        if let Some(id) = invoice_id {
            let mut invoice_subs = self
                .invoice_subs
                .lock()
                .unwrap_or_else(PoisonError::into_inner);

            invoice_subs
                .get_mut(&id)
                .and_then(|map| map.remove(&sender_id));
        } else {
            let mut global_subs = self
                .global_subs
                .lock()
                .unwrap_or_else(PoisonError::into_inner);

            global_subs.remove(&sender_id);
        }
    }
}

#[derive(Hash, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
struct SenderId(u128);

impl SenderId {
    fn new() -> SenderId {
        SenderId(rand::random())
    }
}

/// An error occurring while receiving invoice updates.
#[derive(Error, Debug)]
pub enum SubscriberError {
    /// Timed out before receiving update.
    #[error("subscriber recv timeout: {0}")]
    RecvTimeout(#[from] Elapsed),
    /// Subscriber is empty or disconnected.
    #[error("subscriber try recv failed: {0}")]
    TryRecv(#[from] TryRecvError),
}
