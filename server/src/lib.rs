//! # `libacceptxmr_server`: Everything needed for a standalone monero payment gateway.
//! `libacceptxmr_server` is a batteries-included monero payment gateway library
//! built around the general purpose `AcceptXMR` library.
//!
//! This library is intended for use by the AcceptXMR-Server binary, and is not
//! intended to be used on its own.

#![warn(clippy::pedantic)]
#![warn(missing_docs)]
#![warn(clippy::cargo)]
#![allow(clippy::module_name_repetitions)]

mod config;
pub mod logging;
mod server;

use acceptxmr::{storage::stores::Sqlite, PaymentGateway, PaymentGatewayBuilder};
use futures::try_join;
use log::{debug, error, info, warn};
use secrecy::ExposeSecret;

use crate::{
    config::Config,
    logging::{init_logger, set_verbosity},
    server::{
        api::{external, internal},
        new_server,
    },
};

/// Start a standalone payment gateway.
pub async fn entrypoint() {
    init_logger();
    let config = load_config();
    set_verbosity(config.logging);

    let payment_gateway = build_gateway(&config);
    info!("Payment gateway created.");

    let gateway_clone = spawn_gateway(payment_gateway).await;

    run_server(config, gateway_clone).await;
}

/// Loads config.
///
/// # Panics
///
/// Panics if the config could not be read or validated.
#[must_use]
pub fn load_config() -> Config {
    let config = Config::read().expect("failed to read config");
    config.validate();

    config
}

/// Build a payment gateway from provided config.
///
/// # Panics
///
/// Panics if the payment gateway could not be built.
pub fn build_gateway(config: &Config) -> PaymentGateway<Sqlite> {
    std::fs::create_dir_all(&config.database.path).expect("failed to create DB dir");
    let db_path = config
        .database
        .path
        .canonicalize()
        .expect("could not determine absolute database path")
        .join("database");
    let db_path_str = db_path.to_str().expect("failed to cast DB path to string");

    let private_view_key = config
        .wallet
        .private_viewkey
        .as_ref()
        .expect("private viewkey must be configured");
    let primary_address = config
        .wallet
        .primary_address
        .expect("primary address must be configured");

    let store = Sqlite::new(db_path_str, "invoices", "output keys", "height")
        .expect("failed to open invoice store");
    let mut payment_gateway_builder = PaymentGatewayBuilder::new(
        private_view_key.expose_secret().clone(),
        primary_address.to_string(),
        store,
    )
    .account_index(config.wallet.account_index)
    .daemon_url(config.daemon.url.to_string());

    // Use daemon login if one was configured.
    if let Some(login) = config.daemon.login.as_ref() {
        payment_gateway_builder = payment_gateway_builder.daemon_login(
            login.username.clone(),
            login
                .password
                .as_ref()
                .map(|pass| pass.expose_secret().clone())
                .unwrap_or_default(),
        );
    }

    // Use restore height if one was configured.
    if let Some(restore_height) = config.wallet.restore_height {
        payment_gateway_builder = payment_gateway_builder.initial_height(restore_height);
    }

    payment_gateway_builder
        .build()
        .expect("failed to build payment gateway")
}

/// Run the payment gateway and spawn a thread to monitor it, returning a clone
/// of the running gateway.
///
/// # Panics
///
/// Panics if the payment gateway could not be run.
pub async fn spawn_gateway(payment_gateway: PaymentGateway<Sqlite>) -> PaymentGateway<Sqlite> {
    payment_gateway
        .run()
        .await
        .expect("failed to run payment gateway");
    info!("Payment gateway running.");

    let gateway_clone = payment_gateway.clone();

    // Watch for invoice updates and deal with them accordingly.
    std::thread::spawn(move || {
        // Watch all invoice updates.
        let mut subscriber = payment_gateway.subscribe_all();
        loop {
            let Some(invoice) = subscriber.blocking_recv() else {
                // TODO: Should this attempt to restart instead?
                panic!("Blockchain scanner crashed!")
            };
            // If it's confirmed or expired, we probably shouldn't bother tracking it
            // anymore.
            if (invoice.is_confirmed() && invoice.creation_height() < invoice.current_height())
                || invoice.is_expired()
            {
                debug!(
                    "Invoice to index {} is either confirmed or expired. Removing invoice now",
                    invoice.index()
                );
                if let Err(e) = payment_gateway.remove_invoice(invoice.id()) {
                    error!("Failed to remove fully confirmed invoice: {}", e);
                };
            }
        }
    });

    gateway_clone
}

/// Start the internal and external HTTP servers.
///
/// # Panics
///
/// Panics if one of the servers could not be run, or if they encounter an
/// unrecoverable error while running.
pub async fn run_server(config: Config, payment_gateway: PaymentGateway<Sqlite>) {
    let external_server = new_server(&config.external_api, external, payment_gateway.clone())
        .expect("failed to start external API server");
    let internal_server = new_server(&config.internal_api, internal, payment_gateway)
        .expect("failed to start internal API server");
    try_join! {external_server, internal_server}.unwrap();
}
