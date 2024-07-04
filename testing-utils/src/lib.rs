#![allow(missing_docs)]
#![allow(clippy::missing_panics_doc)]

mod daemon;
mod invoice;

pub use daemon::MockDaemon;
pub use invoice::MockInvoice;
use tempfile::Builder;
use tracing_subscriber::{filter::LevelFilter, prelude::*, EnvFilter};

pub const PRIVATE_VIEW_KEY: &str =
    "ad2093a5705b9f33e6f0f0c1bc1f5f639c756cdfc168c8f2ac6127ccbdab3a03";
pub const PRIMARY_ADDRESS: &str =
    "4613YiHLM6JMH4zejMB2zJY5TwQCxL8p65ufw8kBP5yxX9itmuGLqp1dS4tkVoTxjyH3aYhYNrtGHbQzJQP5bFus3KHVdmf";

#[must_use]
pub fn new_temp_dir() -> String {
    Builder::new()
        .prefix("temp_db_")
        .rand_bytes(16)
        .tempdir()
        .expect("failed to generate temporary directory")
        .path()
        .to_str()
        .expect("failed to get temporary directory path")
        .to_string()
}

/// Initialize the logging implementation.
pub fn init_logger() {
    let filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::DEBUG.into())
        .from_env_lossy();
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_test_writer()
        .with_filter(filter);
    let _ = tracing_subscriber::registry().with(fmt_layer).try_init();
}
