//! `AcceptXMR` can use a storage layer of your choosing. Consumers of this
//! library can use one of the existing storage layers found in [`stores`], or
//! can implement the [`Storage`] trait themselves for a custom storage
//! solution.

mod height_storage;
mod invoice_storage;
mod output_key_storage;
pub mod stores;

pub use height_storage::HeightStorage;
pub use invoice_storage::InvoiceStorage;
use log::error;
pub use output_key_storage::{OutputId, OutputKeyStorage, OutputPubKey};
use thiserror::Error;
use tokio::sync::{
    mpsc::{self},
    oneshot,
};

use crate::{Invoice, InvoiceId, SubIndex};

/// A supertrait of all necessary storage traits.
pub trait Storage: InvoiceStorage + OutputKeyStorage + HeightStorage {
    /// Error type for the storage layer.
    type Error: std::error::Error + Send + 'static;

    /// Flush all changes to disk. This method should be manually implemented
    /// for any storage layer that does not automatically flush on write. The
    /// default implementation does nothing.
    ///
    /// # Errors
    ///
    /// Returns an error if flush does not succeed.
    fn flush(&self) -> Result<(), <Self as Storage>::Error> {
        Ok(())
    }
}

/// The storage manager takes messages from a channel and runs the corresponding
/// storage method. This allows the blocking IO to be performed on a dedicated
/// thread.
struct Manager<S: Storage> {
    store: S,
    receiver: mpsc::Receiver<Method<S>>,
}

impl<S: Storage> Manager<S> {
    fn handle(&mut self, message: Method<S>) {
        match message {
            // Invoice storage methods.
            Method::InsertInvoice { invoice, response } => {
                let id = invoice.id();
                let result = InvoiceStorage::insert(&mut self.store, invoice);
                if response.send(result).is_err() {
                    error!(
                        "Failed to send InsertInvoice response to storage client. Invoice ID: {id}"
                    );
                };
            }
            Method::RemoveInvoice { id, response } => {
                let invoice = InvoiceStorage::remove(&mut self.store, id);
                if response.send(invoice).is_err() {
                    error!(
                        "Failed to send RemoveInvoice response to storage client. Invoice ID: {id}"
                    );
                };
            }
            Method::UpdateInvoice { invoice, response } => {
                let id = invoice.id();
                let result = InvoiceStorage::update(&mut self.store, invoice);
                if response.send(result).is_err() {
                    error!(
                        "Failed to send UpdateInvoice response to storage client. Invoice ID: {id}"
                    );
                };
            }
            Method::GetInvoice { id, response } => {
                let invoice = InvoiceStorage::get(&self.store, id);
                if response.send(invoice).is_err() {
                    error!(
                        "Failed to send GetInvoice response to storage client. Invoice ID: {id}"
                    );
                };
            }
            Method::GetInvoiceIds { response } => {
                let invoice_ids = InvoiceStorage::get_ids(&self.store);
                if response.send(invoice_ids).is_err() {
                    error!("Failed to send GetInvoiceIds response to storage client.");
                };
            }
            Method::ContainsSubIndex { index, response } => {
                if response.send(self.store.contains_sub_index(index)).is_err() {
                    error!(
                        "Failed to send ContainsSubIndex response to storage client. Index: {}",
                        index
                    );
                }
            }
            Method::ForEachInvoice { f, response } => {
                let result = self.store.try_for_each(f);
                if response.send(result).is_err() {
                    error!("Failed to send ForEachInvoice response to storage client.");
                };
            }
            Method::LowestInvoiceHeight(response) => {
                if response.send(self.store.lowest_height()).is_err() {
                    error!("Failed to send LowestInvoiceHeight response to storage client.");
                };
            }

            Method::GetHeight(response) => {
                if response.send(HeightStorage::get(&self.store)).is_err() {
                    error!("Failed to send GetHeight response to storage client.");
                };
            }
            Method::UpsertHeight { height, response } => {
                if response
                    .send(HeightStorage::upsert(&mut self.store, height))
                    .is_err()
                {
                    error!("Failed to send UpsertHeight response to storage client.");
                };
            }

            Method::GetOutputKeyId { key, response } => {
                if response
                    .send(OutputKeyStorage::get(&self.store, key))
                    .is_err()
                {
                    error!("Failed to send GetOutputKeyId response to storage client.");
                };
            }
            Method::InsertOutputKey {
                key,
                output_id,
                response,
            } => {
                if response
                    .send(OutputKeyStorage::insert(&mut self.store, key, output_id))
                    .is_err()
                {
                    error!("Failed to send InsertOutputKey response to storage client.");
                };
            }

            Method::Flush(response) => {
                if response.send(self.store.flush()).is_err() {
                    error!("Failed to send Flush response to storage client.");
                };
            }
        }
    }
}

enum Method<S: Storage> {
    InsertInvoice {
        invoice: Invoice,
        response: oneshot::Sender<Result<(), <S as InvoiceStorage>::Error>>,
    },
    RemoveInvoice {
        id: InvoiceId,
        response: oneshot::Sender<Result<Option<Invoice>, <S as InvoiceStorage>::Error>>,
    },
    UpdateInvoice {
        invoice: Invoice,
        response: oneshot::Sender<Result<Option<Invoice>, <S as InvoiceStorage>::Error>>,
    },
    GetInvoice {
        id: InvoiceId,
        response: oneshot::Sender<Result<Option<Invoice>, <S as InvoiceStorage>::Error>>,
    },
    GetInvoiceIds {
        response: oneshot::Sender<Result<Vec<InvoiceId>, <S as InvoiceStorage>::Error>>,
    },
    ContainsSubIndex {
        index: SubIndex,
        response: oneshot::Sender<Result<bool, <S as InvoiceStorage>::Error>>,
    },
    ForEachInvoice {
        f: Box<ForEachClosure<S>>,
        response: oneshot::Sender<Result<(), <S as InvoiceStorage>::Error>>,
    },
    LowestInvoiceHeight(oneshot::Sender<Result<Option<u64>, <S as InvoiceStorage>::Error>>),
    GetHeight(oneshot::Sender<Result<Option<u64>, <S as HeightStorage>::Error>>),
    UpsertHeight {
        height: u64,
        response: oneshot::Sender<Result<Option<u64>, <S as HeightStorage>::Error>>,
    },
    GetOutputKeyId {
        key: OutputPubKey,
        response: oneshot::Sender<Result<Option<OutputId>, <S as OutputKeyStorage>::Error>>,
    },
    InsertOutputKey {
        key: OutputPubKey,
        output_id: OutputId,
        response: oneshot::Sender<Result<(), <S as OutputKeyStorage>::Error>>,
    },
    Flush(oneshot::Sender<Result<(), <S as Storage>::Error>>),
}

type ForEachClosure<S> = dyn FnMut(Result<Invoice, <S as InvoiceStorage>::Error>) -> Result<(), <S as InvoiceStorage>::Error>
    + Send;

pub(crate) struct Client<S: Storage>(mpsc::Sender<Method<S>>);

impl<S: Storage + 'static> Client<S> {
    pub(crate) fn new(store: S) -> Self {
        let (sender, receiver) = mpsc::channel(64);
        let mut manager = Manager { store, receiver };

        tokio::spawn(async move {
            while let Some(message) = manager.receiver.recv().await {
                manager.handle(message);
            }
        });

        Self(sender)
    }

    pub(crate) async fn insert_invoice(&self, invoice: Invoice) -> Result<(), StorageError> {
        let (sender, receiver) = oneshot::channel();
        self.0
            .send(Method::InsertInvoice {
                invoice,
                response: sender,
            })
            .await
            .map_err(|e| StorageError::Send(Box::new(e)))?;
        let response = receiver.await.map_err(|_| StorageError::Receive)?;
        response.map_err(|e| StorageError::Internal(Box::new(e)))
    }

    pub(crate) async fn remove_invoice(
        &self,
        id: InvoiceId,
    ) -> Result<Option<Invoice>, StorageError> {
        let (sender, receiver) = oneshot::channel();
        self.0
            .send(Method::RemoveInvoice {
                id,
                response: sender,
            })
            .await
            .map_err(|e| StorageError::Send(Box::new(e)))?;
        let response = receiver.await.map_err(|_| StorageError::Receive)?;
        response.map_err(|e| StorageError::Internal(Box::new(e)))
    }

    pub(crate) async fn update_invoice(
        &self,
        invoice: Invoice,
    ) -> Result<Option<Invoice>, StorageError> {
        let (sender, receiver) = oneshot::channel();
        self.0
            .send(Method::UpdateInvoice {
                invoice,
                response: sender,
            })
            .await
            .map_err(|e| StorageError::Send(Box::new(e)))?;
        let response = receiver.await.map_err(|_| StorageError::Receive)?;
        response.map_err(|e| StorageError::Internal(Box::new(e)))
    }

    pub(crate) async fn get_invoice(&self, id: InvoiceId) -> Result<Option<Invoice>, StorageError> {
        let (sender, receiver) = oneshot::channel();
        self.0
            .send(Method::GetInvoice {
                id,
                response: sender,
            })
            .await
            .map_err(|e| StorageError::Send(Box::new(e)))?;
        let response = receiver.await.map_err(|_| StorageError::Receive)?;
        response.map_err(|e| StorageError::Internal(Box::new(e)))
    }

    pub(crate) async fn get_invoice_ids(&self) -> Result<Vec<InvoiceId>, StorageError> {
        let (sender, receiver) = oneshot::channel();
        self.0
            .send(Method::GetInvoiceIds { response: sender })
            .await
            .map_err(|e| StorageError::Send(Box::new(e)))?;
        let response = receiver.await.map_err(|_| StorageError::Receive)?;
        response.map_err(|e| StorageError::Internal(Box::new(e)))
    }

    pub(crate) async fn contains_sub_index(&self, index: SubIndex) -> Result<bool, StorageError> {
        let (sender, receiver) = oneshot::channel();
        self.0
            .send(Method::ContainsSubIndex {
                index,
                response: sender,
            })
            .await
            .map_err(|e| StorageError::Send(Box::new(e)))?;
        let response = receiver.await.map_err(|_| StorageError::Receive)?;
        response.map_err(|e| StorageError::Internal(Box::new(e)))
    }

    pub(crate) async fn try_for_each_invoice<F>(&self, f: F) -> Result<(), StorageError>
    where
        F: FnMut(
                Result<Invoice, <S as InvoiceStorage>::Error>,
            ) -> Result<(), <S as InvoiceStorage>::Error>
            + Send
            + 'static,
    {
        let (sender, receiver) = oneshot::channel();
        self.0
            .send(Method::ForEachInvoice {
                f: Box::new(f),
                response: sender,
            })
            .await
            .map_err(|e| StorageError::Send(Box::new(e)))?;
        let response = receiver.await.map_err(|_| StorageError::Receive)?;
        response.map_err(|e| StorageError::Internal(Box::new(e)))
    }

    pub(crate) async fn lowest_invoice_height(&self) -> Result<Option<u64>, StorageError> {
        let (sender, receiver) = oneshot::channel();
        self.0
            .send(Method::LowestInvoiceHeight(sender))
            .await
            .map_err(|e| StorageError::Send(Box::new(e)))?;
        let response = receiver.await.map_err(|_| StorageError::Receive)?;
        response.map_err(|e| StorageError::Internal(Box::new(e)))
    }

    pub(crate) async fn get_height(&self) -> Result<Option<u64>, StorageError> {
        let (sender, receiver) = oneshot::channel();
        self.0
            .send(Method::GetHeight(sender))
            .await
            .map_err(|e| StorageError::Send(Box::new(e)))?;
        let response = receiver.await.map_err(|_| StorageError::Receive)?;
        response.map_err(|e| StorageError::Internal(Box::new(e)))
    }

    pub(crate) async fn upsert_height(&self, height: u64) -> Result<Option<u64>, StorageError> {
        let (sender, receiver) = oneshot::channel();
        self.0
            .send(Method::UpsertHeight {
                height,
                response: sender,
            })
            .await
            .map_err(|e| StorageError::Send(Box::new(e)))?;
        let response = receiver.await.map_err(|_| StorageError::Receive)?;
        response.map_err(|e| StorageError::Internal(Box::new(e)))
    }

    pub(crate) async fn get_output_key_id(
        &self,
        key: OutputPubKey,
    ) -> Result<Option<OutputId>, StorageError> {
        let (sender, receiver) = oneshot::channel();
        self.0
            .send(Method::GetOutputKeyId {
                key,
                response: sender,
            })
            .await
            .map_err(|e| StorageError::Send(Box::new(e)))?;
        let response = receiver.await.map_err(|_| StorageError::Receive)?;
        response.map_err(|e| StorageError::Internal(Box::new(e)))
    }

    pub(crate) async fn insert_output_key(
        &self,
        key: OutputPubKey,
        output_id: OutputId,
    ) -> Result<(), StorageError> {
        let (sender, receiver) = oneshot::channel();
        self.0
            .send(Method::InsertOutputKey {
                key,
                output_id,
                response: sender,
            })
            .await
            .map_err(|e| StorageError::Send(Box::new(e)))?;
        let response = receiver.await.map_err(|_| StorageError::Receive)?;
        response.map_err(|e| StorageError::Internal(Box::new(e)))
    }

    pub(crate) async fn flush(&self) -> Result<(), StorageError> {
        let (sender, receiver) = oneshot::channel();
        self.0
            .send(Method::Flush(sender))
            .await
            .map_err(|e| StorageError::Send(Box::new(e)))?;
        let response = receiver.await.map_err(|_| StorageError::Receive)?;
        response.map_err(|e| StorageError::Internal(Box::new(e)))
    }
}

impl<S: Storage> Clone for Client<S> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

/// An error occurring while storing or retrieving values from a database.
#[derive(Error, Debug)]
pub enum StorageError {
    /// Failed to send message to the storage manager.
    #[error("failed to send message to the storage manager: {0}")]
    Send(Box<dyn std::error::Error + Send>),
    /// Failed to receive result from the storage manager.
    #[error("failed to receive result from the storage manager")]
    Receive,
    /// An error caused by the database, or some interaction with it.
    #[error(transparent)]
    Internal(Box<dyn std::error::Error + Send>),
}
