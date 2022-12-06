//! Built-in implementors of [`InvoiceStorage`](super::InvoiceStorage) for storing pending invoices.

#[cfg(feature = "in-memory")]
mod in_memory;
#[cfg(feature = "sled")]
mod sled;

#[cfg(feature = "sled")]
pub use super::stores::sled::{Sled, SledStorageError};
#[cfg(feature = "in-memory")]
pub use in_memory::{InMemory, InMemoryStorageError};
