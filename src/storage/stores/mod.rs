//! Built-in implementors of [`InvoiceStorage`](super::InvoiceStorage) for
//! storing pending invoices.

#[cfg(feature = "in-memory")]
mod in_memory;
#[cfg(feature = "sled")]
mod sled;
#[cfg(feature = "sqlite")]
mod sqlite;

#[cfg(feature = "in-memory")]
pub use super::stores::in_memory::{InMemory, InMemoryStorageError};
#[cfg(feature = "sled")]
pub use super::stores::sled::{Sled, SledStorageError};
#[cfg(feature = "sqlite")]
pub use super::stores::sqlite::{Sqlite, SqliteStorageError};
