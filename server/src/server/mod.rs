pub mod api;
mod auth;
mod tls;
mod websocket;

use acceptxmr::{storage::stores::Sqlite, PaymentGateway};
use actix_files::Files;
use actix_web::{
    dev::Server,
    middleware::Condition,
    web::{Data, ServiceConfig},
    App, HttpServer,
};
use actix_web_httpauth::middleware::HttpAuthentication;
use auth::bearer_auth_validator;
use log::{debug, info};
use tls::prepare_tls;
use websocket::WebSocket;

use super::config::ServerConfig;

pub fn new_server<F>(
    server_config: &ServerConfig,
    service: F,
    payment_gateway: PaymentGateway<Sqlite>,
) -> std::io::Result<Server>
where
    F: FnOnce(&mut ServiceConfig) + Copy + Send + 'static,
{
    let bearer_auth_enabled = server_config.token.is_some();
    let shared_payment_gateway = Data::new(payment_gateway);
    let shared_config = Data::new(server_config.clone());
    let static_dir = server_config.static_dir.clone();
    let mut server_builder = HttpServer::new(move || {
        App::new()
            .app_data(shared_payment_gateway.clone())
            .app_data(shared_config.clone())
            .configure(service)
            .service(Files::new("", static_dir.clone()).index_file("index.html"))
            .wrap(Condition::new(
                bearer_auth_enabled,
                HttpAuthentication::bearer(bearer_auth_validator),
            ))
    });
    // Enable TLS.
    server_builder = if let Some(tls) = &server_config.tls {
        info!(
            "Binding with TLS to {}:{}",
            server_config.ipv4, server_config.port
        );
        let rustls_config = prepare_tls(tls);
        // Enable IPv6.
        if let Some(ipv6) = server_config.ipv6 {
            server_builder = server_builder
                .bind_rustls_021((ipv6, server_config.port), rustls_config.clone())?;
            debug!("Bound to: {:?}", server_builder.addrs());
        }
        server_builder.bind_rustls_021((server_config.ipv4, server_config.port), rustls_config)?
    } else {
        info!(
            "Binding without TLS to {}:{}",
            server_config.ipv4, server_config.port
        );
        // Enable IPv6.
        if let Some(ipv6) = server_config.ipv6 {
            server_builder = server_builder.bind((ipv6, server_config.port))?;
            debug!("Bound to: {:?}", server_builder.addrs());
        }
        server_builder.bind((server_config.ipv4, server_config.port))?
    };
    debug!("Bound to: {:?}", server_builder.addrs());
    Ok(server_builder.run())
}
