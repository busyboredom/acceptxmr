//! # `AcceptXMR-Server`: A monero payment gateway.
//! `AcceptXMR-Server` is a batteries-included monero payment gateway built
//! around the `AcceptXMR` library.
//!
//! If your application requires more flexibility than `AcceptXMR-Server`
//! offers, please see the [`AcceptXMR`](../library/) library instead.

use acceptxmr_server::entrypoint;

#[tokio::main]
async fn main() {
    entrypoint().await;
}
