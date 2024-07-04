use std::borrow::Cow;

use acceptxmr::{storage::Storage, InvoiceId, MonerodClient, Subscriber};
use axum::{
    extract::{
        ws::{close_code, CloseFrame, Message, WebSocket},
        Query, State as AxumState, WebSocketUpgrade,
    },
    http::HeaderValue,
    response::{Html, IntoResponse},
    routing::get,
    Json, Router,
};
use futures::{SinkExt, StreamExt};
use hyper::{http::header::CACHE_CONTROL, StatusCode};
use log::{debug, error};
use serde_json::json;
use tera::Context;
use utoipa::{openapi::OpenApi, OpenApi as _};

use super::{
    types::invoice_id::{InvoiceIdPayload, InvoiceIdQuery},
    ApiError,
};
use crate::server::{
    api::{templating::external_templates, Base64InvoiceId, InvoiceUpdate},
    State,
};

#[derive(utoipa::OpenApi)]
#[openapi(
    paths(
        get_invoice_status,
        websocket,
        pay
    ),
    components(
        schemas(InvoiceIdQuery,InvoiceUpdate, Base64InvoiceId)
    ),
    tags(
        (name = "External API", description = "AcceptXMR's user-facing API")
    )
)]
struct ApiDoc;

pub(crate) fn external<S: Storage + 'static, M: MonerodClient + 'static>(
    state: State<S, M>,
) -> (Router, OpenApi) {
    (
        Router::new()
            .route("/invoice/ws", get(websocket))
            .route("/invoice", get(get_invoice_status))
            .route("/pay", get(pay))
            .with_state(state),
        ApiDoc::openapi(),
    )
}

/// Get invoice status.
///
/// Get invoice status by ID with query param.
#[utoipa::path(
    get,
    path = "/invoice",
    params(
        InvoiceIdQuery
    ),
    responses(
        (status = 200, description = "Status of invoice", body = InvoiceUpdate)
    )
)]
async fn get_invoice_status<S: Storage + 'static, M: MonerodClient + 'static>(
    query: Query<InvoiceIdQuery>,
    AxumState(state): AxumState<State<S, M>>,
) -> Result<impl IntoResponse, ApiError> {
    let invoice_id = InvoiceId::try_from(query.0)?;
    match state.payment_gateway.get_invoice(invoice_id).await {
        Ok(Some(invoice)) => Ok((
            [(CACHE_CONTROL, HeaderValue::from_static("no-store"))],
            Json(InvoiceUpdate::from(invoice)),
        )),
        Ok(None) => Err(ApiError::InvoiceNotFound(invoice_id)),
        Err(e) => {
            error!("Error getting invoice status: {e}");
            Err(ApiError::AcceptXmr(e))
        }
    }
}

/// Subscribe to an invoice.
///
/// Subscribe to an invoice's updates via websocket. Invoice updates sent over
/// the resulting websocket connection will be of type `InvoiceUpdate`.
#[utoipa::path(
    get,
    path = "/invoice/ws",
    params(
        InvoiceIdQuery
    ),
    responses(
        (status = 101)
    )
)]
async fn websocket<S: Storage + 'static, M: MonerodClient + 'static>(
    query: Query<InvoiceIdQuery>,
    ws: WebSocketUpgrade,
    AxumState(state): AxumState<State<S, M>>,
) -> Result<impl IntoResponse, ApiError> {
    let invoice_id = InvoiceId::try_from(query.0)?;
    let Some(subscriber) = state.payment_gateway.subscribe(invoice_id) else {
        debug!("Can't perform websocket upgrade for invoice which doesn't exist: {invoice_id}");
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    Ok(ws.on_upgrade(|socket| handle_websocket(socket, subscriber)))
}

#[allow(clippy::unused_async)]
async fn handle_websocket(socket: WebSocket, mut subscriber: Subscriber) {
    let (mut sender, mut receiver) = socket.split();
    debug!("Opening websocket connection");

    tokio::spawn(async move {
        while let Some(message) = receiver.next().await {
            match message {
                Ok(Message::Ping(_)) => {}
                Ok(Message::Close(cf)) => {
                    let close_message = if let Some(msg) = cf {
                        format!(": {}", msg.reason)
                    } else {
                        String::default()
                    };
                    debug!("Websocket client closed the connection{}.", close_message);
                }
                Ok(m) => {
                    debug!("Unexpected message from websocket client: {m:?}");
                }
                Err(e) => {
                    error!("Error receiving websocket message: {e}");
                }
            }
        }
    });

    tokio::spawn(async move {
        while let Some(invoice) = subscriber.recv().await {
            match sender
                .send(Message::Text(
                    json!(InvoiceUpdate::from(invoice.clone())).to_string(),
                ))
                .await
            {
                Ok(()) => {}
                Err(e) => {
                    error!("Error sending invoice update: {e}");
                }
            }
            // If the invoice is confirmed or expired, stop checking for updates.
            if invoice.is_confirmed() {
                if let Err(e) = sender
                    .send(Message::Close(Some(CloseFrame {
                        code: close_code::NORMAL,
                        reason: Cow::Borrowed("Invoice Complete"),
                    })))
                    .await
                {
                    error!("Error sending websocket close message after invoice confirmation: {e}");
                };
                if let Err(e) = sender.close().await {
                    error!("Error closing websocket after invoice confirmation: {e}");
                };
            } else if invoice.is_expired() {
                if let Err(e) = sender
                    .send(Message::Close(Some(CloseFrame {
                        code: close_code::NORMAL,
                        reason: Cow::Borrowed("Invoice Expired"),
                    })))
                    .await
                {
                    error!("Error sending websocket close message after invoice expiration: {e}");
                };
                if let Err(e) = sender.close().await {
                    error!("Error closing websocket after invoice expiration: {e}");
                };
            }
        }
    });
}

/// Payment UI.
///
/// Returns a simple UI prompting the user to pay.
#[utoipa::path(
    get,
    path = "/pay",
    params(
        InvoiceIdQuery
    ),
    responses(
        (status = 200, description = "Payment UI")
    )
)]
async fn pay<S: Storage + 'static, M: MonerodClient + 'static>(
    query: Query<InvoiceIdQuery>,
    AxumState(state): AxumState<State<S, M>>,
) -> Result<impl IntoResponse, ApiError> {
    let invoice_id = InvoiceId::try_from(query.0)?;
    let templates = external_templates(
        state
            .config
            .static_dir
            .join("**/*.html")
            .to_string_lossy()
            .as_ref(),
    );
    match state.payment_gateway.get_invoice(invoice_id).await {
        Ok(Some(invoice)) => Ok((
            StatusCode::OK,
            [(CACHE_CONTROL, HeaderValue::from_static("no-store"))],
            {
                let payment_page = templates
                    .render(
                        "pay.html",
                        &Context::from_serialize(InvoiceUpdate::from(invoice))
                            .inspect_err(|e| error!("Failed to build templating context: {e}"))?,
                    )
                    .inspect_err(|e| error!("Failed to render template: {e}"))?;
                Html(payment_page)
            },
        )),
        Ok(None) => Ok((
            StatusCode::NOT_FOUND,
            [(CACHE_CONTROL, HeaderValue::from_static("no-store"))],
            {
                let missing_invoice_page = templates
                    .render(
                        "missing-invoice.html",
                        &Context::from_serialize(InvoiceIdPayload::from(invoice_id))
                            .inspect_err(|e| error!("Failed to build templating context: {e}"))?,
                    )
                    .inspect_err(|e| error!("Failed to render template: {e}"))?;
                Html(missing_invoice_page)
            },
        )),
        Err(e) => Ok((
            StatusCode::INTERNAL_SERVER_ERROR,
            [(CACHE_CONTROL, HeaderValue::from_static("no-store"))],
            {
                let mut context = Context::new();
                context.insert("error", &e.to_string());
                let error_page = templates
                    .render("error.html", &context)
                    .inspect_err(|e| error!("Failed to render template: {e}"))?;
                Html(error_page)
            },
        )),
    }
}

#[cfg(test)]
mod test {
    use acceptxmr::{storage::stores::InMemory, MonerodMockClient, PaymentGatewayBuilder};
    use axum::{body::Body, http::Request};
    use http_body_util::{BodyExt, Empty};
    use hyper::{header, StatusCode};
    use serde_json::json;
    use testing_utils::{init_logger, PRIMARY_ADDRESS, PRIVATE_VIEW_KEY};
    use tower::ServiceExt;

    use super::external;
    use crate::{
        config::ServerConfig,
        server::{api::internal, state::State},
    };

    #[tokio::test]
    async fn get_invoice_status() {
        init_logger();

        let payment_gateway = PaymentGatewayBuilder::new(
            PRIVATE_VIEW_KEY.to_string(),
            PRIMARY_ADDRESS.to_string(),
            InMemory::new(),
        )
        .seed(0)
        .build_with_mock_daemon()
        .await
        .unwrap();
        let (internal_api, _) = internal(State::<InMemory, MonerodMockClient>::new(
            payment_gateway.clone(),
            ServerConfig::default(),
        ));
        let (external_api, _) = external(State::<InMemory, MonerodMockClient>::new(
            payment_gateway,
            ServerConfig::default(),
        ));

        // `Router` implements `tower::Service<Request<Body>>` so we can
        // call it like any tower service, no need to run an HTTP server.
        let response = internal_api
            .oneshot(
                Request::post("/invoice")
                    .header(header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
                    .body(Body::from(
                        serde_json::to_vec(&json!({
                            "piconeros_due": 1_000_000,
                            "confirmations_required": 2,
                            "expiration_in": 10,
                            "order": "large pizza",
                            "callback": "https://example.com/success?=largepizza",
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            &String::from_utf8_lossy(&body[..]),
            "{\"invoice_id\":\"AAAAAAAAAEkAAAAAACXOWQ\"}"
        );

        let response = external_api
            .oneshot(
                (Request::get("/invoice?id=AAAAAAAAAEkAAAAAACXOWQ"))
                    .body(Empty::new())
                    .unwrap(),
            )
            .await
            .unwrap();

        dbg!(response.body());
        assert_eq!(response.status(), StatusCode::OK);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body[..]).unwrap(),
            json!(
                {
                    "id":"AAAAAAAAAEkAAAAAACXOWQ",
                    "address":"84Gv7pf9wJhUS1pK7Kn7Fw2UScnKjdVnxRQfQMC3tsuZbMZkVKiUBrrJ8UPsztJQUXiFdEb1kcsD33bJy98gUB2g4pvirxc",
                    "uri": r"monero:84Gv7pf9wJhUS1pK7Kn7Fw2UScnKjdVnxRQfQMC3tsuZbMZkVKiUBrrJ8UPsztJQUXiFdEb1kcsD33bJy98gUB2g4pvirxc?tx_amount=0.000001",
                    "amount_requested":1_000_000,
                    "amount_paid":0,
                    "confirmations_required":2,
                    "confirmations":None::<u64>,
                    "expiration_in":10,
                    "current_height":0,
                    "order":"large pizza",
                    "callback":r"https://example.com/success?=largepizza"
                }
            )
        );
    }
}
