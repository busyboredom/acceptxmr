pub mod api;
mod auth;
mod state;
mod tls;

use std::{
    io::Error as IoError,
    net::{SocketAddr, SocketAddrV4, SocketAddrV6},
    pin::Pin,
};

use acceptxmr::{storage::stores::Sqlite, PaymentGateway};
use axum::{extract::Request, Router};
use futures_util::pin_mut;
use hyper::body::Incoming;
use hyper_util::rt::{TokioExecutor, TokioIo};
use log::{debug, error, info};
use state::State;
use tls::prepare_tls;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    join,
    net::TcpListener,
};
use tokio_rustls::TlsAcceptor;
use tower::Service;
use tower_http::{services::ServeDir, validate_request::ValidateRequestHeaderLayer};
use utoipa::openapi::{
    security::{HttpAuthScheme, HttpBuilder, SecurityScheme},
    OpenApi, SecurityRequirement,
};
use utoipa_swagger_ui::SwaggerUi;

use super::config::ServerConfig;
use crate::server::auth::MaybeBearer;

pub(crate) async fn new_server<F>(
    server_config: ServerConfig,
    api: F,
    payment_gateway: PaymentGateway<Sqlite>,
) -> std::io::Result<Server>
where
    F: Fn(State) -> (Router, OpenApi),
{
    let static_dir = server_config.static_dir.clone();

    let state = State::new(payment_gateway, server_config.clone());
    let (router, mut api_doc) = api(state);

    match &server_config.token {
        Some(_token) => {
            if let Some(schema) = api_doc.components.as_mut() {
                schema.add_security_scheme(
                    "bearer",
                    SecurityScheme::Http(HttpBuilder::new().scheme(HttpAuthScheme::Bearer).build()),
                );
            };
            api_doc.security = Some(vec![SecurityRequirement::new("bearer", ["invoice"])]);
        }
        None => {}
    };

    let router = Router::new()
        .merge(router)
        .fallback_service(ServeDir::new(static_dir))
        .layer(ValidateRequestHeaderLayer::custom(MaybeBearer::new(
            server_config.token,
        )))
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", api_doc));

    debug!("Binding to {}:{}", server_config.ipv4, server_config.port);
    let tcp_v4_listener: TcpListener = TcpListener::bind::<SocketAddrV4>(SocketAddrV4::new(
        server_config.ipv4,
        server_config.port,
    ))
    .await?;
    if let Ok(v4_addr) = tcp_v4_listener.local_addr() {
        info!("Bound to: {:?}", v4_addr);
    }
    let tcp_v6_listener = if let Some(ipv6) = server_config.ipv6 {
        debug!("Binding to {}:{}", ipv6, server_config.port);
        let listener =
            TcpListener::bind::<SocketAddrV6>(SocketAddrV6::new(ipv6, server_config.port, 0, 0))
                .await?;
        if let Ok(v6_addr) = listener.local_addr() {
            info!("Bound to: {:?}", v6_addr);
        }
        Some(listener)
    } else {
        None
    };

    let tls_acceptor = if let Some(tls) = &server_config.tls {
        let rustls_config = prepare_tls(tls);
        Some(TlsAcceptor::from(rustls_config))
    } else {
        None
    };

    Ok(Server {
        ipv4: tcp_v4_listener,
        ipv6: tcp_v6_listener,
        tls: tls_acceptor,
        router,
    })
}

pub(crate) struct Server {
    ipv4: TcpListener,
    ipv6: Option<TcpListener>,
    tls: Option<TlsAcceptor>,
    router: Router,
}

impl Server {
    pub(crate) async fn serve(self) {
        if let Some(ipv6) = &self.ipv6 {
            join!(self.serve_inner(&self.ipv4), self.serve_inner(ipv6));
        } else {
            self.serve_inner(&self.ipv4).await;
        }
    }

    async fn serve_inner(&self, listener: &TcpListener) {
        pin_mut!(listener);
        loop {
            let tower_service = self.router.clone();
            let tls_acceptor = self.tls.clone();

            // Wait for new tcp connection.
            // TODO: Bound the number of open connections somehow.
            let (cnx, addr) = listener.accept().await.unwrap();

            tokio::spawn(async move {
                // Wait for tls handshake to happen.
                let stream: TokioIo<Pin<Box<dyn TokioReadWrite>>> = if let Some(tls) = tls_acceptor
                {
                    match tls.accept(cnx).await {
                        Ok(s) => TokioIo::new(Box::pin(s)),
                        Err(e) => {
                            error!("Error during tls handshake connection from {}: {e}", addr);
                            return;
                        }
                    }
                } else {
                    TokioIo::new(Box::pin(cnx))
                };

                // Hyper also has its own `Service` trait and doesn't use tower. We can use
                // `hyper::service::service_fn` to create a hyper `Service` that calls our app
                // through `tower::Service::call`.
                let hyper_service =
                    hyper::service::service_fn(move |request: Request<Incoming>| {
                        // We have to clone `tower_service` because hyper's `Service` uses `&self`
                        // whereas tower's `Service` requires `&mut self`.
                        //
                        // We don't need to call `poll_ready` since `Router` is always ready.
                        tower_service.clone().call(request)
                    });

                let ret = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new())
                    .serve_connection_with_upgrades(stream, hyper_service)
                    .await;

                if let Err(err) = ret {
                    error!("Error serving connection from {}: {}", addr, err);
                }
            });
        }
    }

    pub(crate) fn ipv4_address(&self) -> Result<SocketAddr, IoError> {
        self.ipv4.local_addr()
    }
}

trait TokioReadWrite: AsyncRead + AsyncWrite + Send {}

impl<T> TokioReadWrite for T where T: AsyncRead + AsyncWrite + Send {}
