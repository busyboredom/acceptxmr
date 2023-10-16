//! Logging utilities for `AcceptXMR` Server.

use log::LevelFilter;

use crate::config::LoggingConfig;

/// Initialize the logging implementation. Defaults to `Trace` verbosity for
/// `AcceptXMR` and `Warn` for dependencies.
pub fn init_logger() {
    env_logger::builder()
        .filter_level(LevelFilter::Warn)
        .filter_module("acceptxmr", LevelFilter::Trace)
        .filter_module("acceptxmr_server", LevelFilter::Trace)
        .init();
}

/// Set verbosity to one of:
/// * Trace
/// * Debug
/// * Info
/// * Error
/// * Warn
pub fn set_verbosity(config: LoggingConfig) {
    log::set_max_level(config.verbosity);
}
