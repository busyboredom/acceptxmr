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
pub use output_key_storage::{OutputId, OutputKeyStorage, OutputPubKey};

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
