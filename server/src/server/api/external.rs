use std::borrow::Cow;

use acceptxmr::Subscriber;
use axum::{
    debug_handler,
    extract::{
        ws::{close_code, CloseFrame, Message, WebSocket},
        Query, State as AxumState, WebSocketUpgrade,
    },
    http::HeaderValue,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use futures::{SinkExt, StreamExt};
use hyper::{http::header::CACHE_CONTROL, StatusCode};
use log::{debug, error};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use super::{ApiError, InvoiceIdPayload};
use crate::server::{api::InvoiceUpdate, State};

#[derive(OpenApi)]
#[openapi(
    paths(
        get_invoice_status,
        websocket,
    ),
    components(
        schemas(InvoiceIdPayload)
    ),
    tags(
        (name = "External API", description = "AcceptXMR's user-facing API")
    )
)]
struct ApiDoc;

pub(crate) fn external(state: State) -> Router {
    Router::new()
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi()))
        .route("/invoice", get(get_invoice_status))
        .route("/invoice/ws", get(websocket))
        .with_state(state)
}

/// Get invoice status.
///
/// Get invoice status by ID with query param.
#[utoipa::path(
    get,
    path = "/invoice",
    params(
        InvoiceIdPayload
    ),
    responses(
        (status = 200, description = "Status of invoice", body = InvoiceUpdate)
    )
)]
#[debug_handler(state = State)]
async fn get_invoice_status(
    query: Query<InvoiceIdPayload>,
    AxumState(state): AxumState<State>,
) -> Result<impl IntoResponse, ApiError> {
    match state.payment_gateway.get_invoice(query.invoice_id).await {
        Ok(Some(invoice)) => Ok((
            [(CACHE_CONTROL, HeaderValue::from_static("no-store"))],
            Json(InvoiceUpdate::from(invoice)),
        )),
        Ok(None) => Err(ApiError::InvoiceNotFound(query.invoice_id)),
        Err(e) => {
            error!("Error getting invoice status: {e}");
            Err(ApiError::AcceptXmr(e))
        }
    }
}

/// Subscribe to an invoice.
///
/// Subscribe to an invoice's updates via websocket.
#[utoipa::path(
    get,
    path = "/invoice/ws",
    params(
        InvoiceIdPayload
    ),
    responses(
        (status = 200, description = "Status of invoice", body = InvoiceUpdate) // TODO: What does this actually return?
    )
)]
#[debug_handler(state = State)]
async fn websocket(
    query: Query<InvoiceIdPayload>,
    ws: WebSocketUpgrade,
    AxumState(state): AxumState<State>,
) -> Result<impl IntoResponse, ApiError> {
    let Some(subscriber) = state.payment_gateway.subscribe(query.invoice_id) else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    Ok(ws.on_upgrade(|socket| handle_websocket(socket, subscriber)))
}

#[allow(clippy::unused_async)]
async fn handle_websocket(socket: WebSocket, mut subscriber: Subscriber) {
    let (mut sender, mut receiver) = socket.split();

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
                .send(Message::Text(Json(invoice.clone()).to_string()))
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
