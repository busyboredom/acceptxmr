//! # `AcceptXMR-Server`: A monero payment gateway.
//! `AcceptXMR-Server` is a batteries-included monero payment gateway built
//! around the `AcceptXMR` library.
//!
//! If your application requires more flexibility than `AcceptXMR-Server`
//! offers, please see the [`AcceptXMR`](../library/) library instead.

#![warn(clippy::pedantic)]
#![warn(missing_docs)]
#![warn(clippy::cargo)]
#![allow(clippy::module_name_repetitions)]

use acceptxmr_server::entrypoint;

#[actix_web::main]
async fn main() {
    entrypoint().await;
}
