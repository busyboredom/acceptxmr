//! # `libacceptxmr_server`: Everything needed for a standalone monero payment gateway.
//! `libacceptxmr_server` is a batteries-included monero payment gateway library
//! built around the general purpose `AcceptXMR` library.
//!
//! This library is intended for use by the AcceptXMR-Server binary, and is not
//! intended to be used on its own.

mod callbacks;
mod config;
pub mod logging;
mod server;

use std::{io::Error as IoError, net::SocketAddr, path::PathBuf, time::Duration};

use acceptxmr::{storage::stores::Sqlite, PaymentGateway, PaymentGatewayBuilder};
use log::{debug, error, info};
use secrecy::ExposeSecret;
use server::Server;
use tokio::{join, try_join};

use crate::{
    callbacks::{CallbackClient, CallbackCommand, CallbackQueue},
    logging::{init_logger, set_verbosity},
    server::{
        api::{external, internal},
        new_server,
    },
};
pub use crate::{config::Config, server::api};

/// Start a standalone payment gateway.
pub async fn entrypoint() {
    init_logger();

    let config_path = Config::get_path();
    let config = load_config(&config_path);
    set_verbosity(config.logging);

    let payment_gateway = build_gateway(&config).await;
    info!("Payment gateway created.");

    let gateway_clone = spawn_gateway(payment_gateway, &config).await;

    let server = build_server(&config, gateway_clone).await;
    debug!("Built AcceptXMR-Server");
    run_server(server).await;
}

/// Loads config.
///
/// # Panics
///
/// Panics if the config could not be read or validated.
#[must_use]
pub fn load_config(path: &PathBuf) -> Config {
    let config = Config::read(path).expect("failed to read config");
    config.validate();

    config
}

/// Build a payment gateway from provided config.
///
/// # Panics
///
/// Panics if the payment gateway could not be built.
pub async fn build_gateway(config: &Config) -> PaymentGateway<Sqlite> {
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
                .clone()
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
        .await
        .expect("failed to build payment gateway")
}

/// Run the payment gateway and spawn a thread to monitor it, returning a clone
/// of the running gateway.
///
/// # Panics
///
/// Panics if the payment gateway could not be run.
pub async fn spawn_gateway(
    payment_gateway: PaymentGateway<Sqlite>,
    config: &Config,
) -> PaymentGateway<Sqlite> {
    payment_gateway
        .run()
        .await
        .expect("failed to run payment gateway");
    info!("Payment gateway running.");

    let gateway_clone = payment_gateway.clone();
    let callback_queue_size = config.callback.queue_size;
    let callback_max_retries = config.callback.max_retries;
    let delete_expired = config.database.delete_expired;

    // Watch for invoice updates and deal with them accordingly.
    tokio::spawn(async move {
        // Watch all invoice updates.
        let mut subscriber = payment_gateway.subscribe_all();
        info!("Subscribed to all invoice updates.");

        // Build http client for callbacks.
        let callback_queue = CallbackQueue::init(
            CallbackClient::default(),
            callback_queue_size,
            callback_max_retries,
        );

        loop {
            let Some(invoice) = subscriber.recv().await else {
                // TODO: Should this attempt to restart instead?
                panic!("Blockchain scanner crashed!")
            };
            debug!("Update for invoice with ID {}:\n{}", invoice.id(), &invoice);

            // Call the callback, if applicable.
            if let Err(e) = callback_queue
                .send(CallbackCommand::Call {
                    invoice: invoice.clone(),
                    delay: Duration::ZERO,
                    retry_count: 0,
                })
                .await
            {
                panic!("Callback queue closed unexpectedly before processing callback for invoice with ID {}. Cause: {}.", invoice.id(), e);
            };

            // If it's expired and not pending confirmation then we probably
            // shouldn't bother tracking it anymore. But only if configured to do so.
            if invoice.is_expired()
                && (invoice.is_confirmed() || !invoice.is_paid())
                && delete_expired
            {
                debug!(
                    "Invoice to index {} is expired. Removing invoice now",
                    invoice.index()
                );
                if let Err(e) = payment_gateway.remove_invoice(invoice.id()).await {
                    error!("Failed to remove expired invoice: {}", e);
                };
            }
        }
    });

    gateway_clone
}

/// Build an instance of `AcceptXmrServer`.
///
/// # Panics
///
/// Panics if the external or internal API servers could not be created (for
/// example, if the specified port could not be bound).
pub async fn build_server(
    config: &Config,
    payment_gateway: PaymentGateway<Sqlite>,
) -> AcceptXmrServer {
    let (external_server, internal_server) = try_join!(
        new_server(
            config.external_api.clone(),
            external,
            payment_gateway.clone(),
        ),
        new_server(config.internal_api.clone(), internal, payment_gateway)
    )
    .expect("failed to start internal or external API server");

    debug!("Built API servers");

    AcceptXmrServer {
        external: external_server,
        internal: internal_server,
    }
}

/// An instance of AcceptXmr-Server.
pub struct AcceptXmrServer {
    external: Server,
    internal: Server,
}

impl AcceptXmrServer {
    /// Return the ipv4 address of the internal API server.
    ///
    /// # Errors
    ///
    /// Returns an IO error if there was an issue getting the address.
    pub fn internal_ipv4_address(&self) -> Result<SocketAddr, IoError> {
        self.internal.ipv4_address()
    }

    /// Return the ipv4 address of the external API server.
    ///
    /// # Errors
    ///
    /// Returns an IO error if there was an issue getting the address.
    pub fn external_ipv4_address(&self) -> Result<SocketAddr, IoError> {
        self.external.ipv4_address()
    }
}

/// Start the internal and external HTTP servers.
///
/// # Panics
///
/// Panics if one of the servers could not be run, or if they encounter an
/// unrecoverable error while running.
pub async fn run_server(server: AcceptXmrServer) {
    let AcceptXmrServer { external, internal } = server;
    let external_handle = tokio::spawn(external.serve());
    let internal_handle = tokio::spawn(internal.serve());
    let _ = join! {external_handle, internal_handle};
}
