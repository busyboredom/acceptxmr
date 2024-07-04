use std::str::FromStr;

use acceptxmr::{storage::Storage, MonerodClient};
use axum::{
    extract::{Query, State as AxumState},
    http::HeaderValue,
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use hyper::{
    http::{header::CACHE_CONTROL, Uri},
    StatusCode,
};
use log::debug;
use serde::Deserialize;
use utoipa::{openapi::OpenApi, OpenApi as _, ToSchema};

use crate::server::{
    api::{
        types::invoice_id::{Base64InvoiceId, InvoiceIdPayload, InvoiceIdQuery},
        ApiError, InvoiceDescription,
    },
    State,
};

#[derive(utoipa::OpenApi)]
#[openapi(
    paths(new_invoice, delete_invoice, invoice_ids),
    components(schemas(InvoiceIdPayload, NewInvoiceParams, Base64InvoiceId)),
    info(
        title = "AcceptXMR Server (Internal)",
        description = "AcceptXMR Server's non user-facing API."
    )
)]
struct ApiDoc;

pub(crate) fn internal<S: Storage + 'static, M: MonerodClient + 'static>(
    state: State<S, M>,
) -> (Router, OpenApi) {
    (
        Router::new()
            .route("/invoice", post(new_invoice))
            .route("/invoice", delete(delete_invoice))
            .route("/invoice/ids", get(invoice_ids))
            //.route("/status", get(status))
            .with_state(state),
        ApiDoc::openapi(),
    )
}

#[derive(Deserialize, ToSchema)]
struct NewInvoiceParams {
    #[schema(example = "1000000")]
    piconeros_due: u64,
    #[schema(example = "1")]
    confirmations_required: u64,
    #[schema(example = "30")]
    expiration_in: u64,
    #[schema(example = "large pizza")]
    order: String,
    #[schema(example = "https://example.com/paid")]
    callback: Option<String>,
}

/// Create a new invoice.
///
/// Create a new invoice with the provided details. Returns the ID of the new
/// invoice.
#[utoipa::path(
    post,
    path = "/invoice",
    tag = "invoice",
    request_body = NewInvoiceParams,
    responses(
        (status = 200, description = "Created a new invoice", body = InvoiceIdPayload)
   )
)]
async fn new_invoice<S: Storage + 'static, M: MonerodClient + 'static>(
    AxumState(state): AxumState<State<S, M>>,
    Json(payload): Json<NewInvoiceParams>,
) -> Result<impl IntoResponse, ApiError> {
    // If there's a callback, check that it is valid.
    if let Some(callback) = &payload.callback {
        let _uri = Uri::from_str(callback).map_err(ApiError::InvalidCallback)?;
    }

    let invoice_id = state
        .payment_gateway
        .new_invoice(
            payload.piconeros_due,
            payload.confirmations_required,
            payload.expiration_in,
            serde_json::to_string(&InvoiceDescription {
                order: payload.order.clone(),
                callback: payload.callback.clone(),
            })
            .map_err(ApiError::DescriptionSerialization)?,
        )
        .await?;
    debug!(
        "Created new invoice successfully. Invoice ID: {}",
        invoice_id
    );
    Ok((
        [(CACHE_CONTROL, HeaderValue::from_static("no-store"))],
        Json(InvoiceIdPayload::from(invoice_id)),
    ))
}

/// Delete an invoice.
///
/// Delete the invoice with the provided ID.
#[utoipa::path(
    delete,
    path = "/invoice",
    tag = "invoice",
    params(
        InvoiceIdQuery
    ),
    responses(
        (status = 200, description = "Deleted the invoice")
   )
)]
async fn delete_invoice<S: Storage + 'static, M: MonerodClient + 'static>(
    AxumState(state): AxumState<State<S, M>>,
    Query(invoice_id): Query<InvoiceIdQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let status = match state
        .payment_gateway
        .remove_invoice(invoice_id.try_into()?)
        .await?
    {
        Some(_) => StatusCode::OK,
        None => StatusCode::NOT_FOUND,
    };
    Ok((
        status,
        [(CACHE_CONTROL, HeaderValue::from_static("no-store"))],
    ))
}

/// List all invoice IDs.
///
/// List invoice IDs of all currently tracked invoices.
#[utoipa::path(
    get,
    path = "/invoice/ids", 
    tag = "invoice",
    responses(
        (status = 200, description = "List of invoice IDs", body = Vec<Base64InvoiceId>)
    )
)]
async fn invoice_ids<S: Storage + 'static, M: MonerodClient + 'static>(
    AxumState(state): AxumState<State<S, M>>,
) -> Result<impl IntoResponse, ApiError> {
    let invoice_ids: Vec<Base64InvoiceId> = state
        .payment_gateway
        .get_invoice_ids()
        .await?
        .into_iter()
        .map(Base64InvoiceId::from)
        .collect();

    Ok((
        [(CACHE_CONTROL, HeaderValue::from_static("no-store"))],
        Json(invoice_ids),
    ))
}

#[cfg(test)]
mod test {
    use acceptxmr::{storage::stores::InMemory, MonerodMockClient, PaymentGatewayBuilder};
    use axum::{body::Body, http::Request};
    use http_body_util::{BodyExt, Empty};
    use hyper::{header, StatusCode};
    use serde_json::json;
    use testing_utils::{init_logger, PRIMARY_ADDRESS, PRIVATE_VIEW_KEY};
    use tower::{Service, ServiceExt};

    use super::internal;
    use crate::{
        api::types::invoice_id::{Base64InvoiceId, InvoiceIdPayload},
        config::ServerConfig,
        server::state::State,
    };

    #[tokio::test]
    async fn new_invoice() {
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
        let (app, _) = internal(State::<InMemory, MonerodMockClient>::new(
            payment_gateway,
            ServerConfig::default(),
        ));

        // `Router` implements `tower::Service<Request<Body>>` so we can
        // call it like any tower service, no need to run an HTTP server.
        let response = app
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

        dbg!(response.body());
        assert_eq!(response.status(), StatusCode::OK);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            &String::from_utf8_lossy(&body[..]),
            "{\"invoice_id\":\"AAAAAAAAAEkAAAAAACXOWQ\"}"
        );
    }

    #[tokio::test]
    async fn delete_invoice() {
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
        let (mut app, _) = internal(State::<InMemory, MonerodMockClient>::new(
            payment_gateway,
            ServerConfig::default(),
        ));

        // `Router` implements `tower::Service<Request<Body>>` so we can
        // call it like any tower service, no need to run an HTTP server.
        let creation_response = app
            .call(
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

        dbg!(creation_response.body());
        assert_eq!(creation_response.status(), StatusCode::OK);

        let body = creation_response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes();
        assert_eq!(
            &String::from_utf8_lossy(&body[..]),
            "{\"invoice_id\":\"AAAAAAAAAEkAAAAAACXOWQ\"}"
        );

        let invoice_id_payload: InvoiceIdPayload = serde_json::from_slice(&body).unwrap();
        let invoice_id = invoice_id_payload.invoice_id;
        let deletion_response = app
            .call(
                Request::delete(format!("/invoice?id={invoice_id}"))
                    .body(Empty::new())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(deletion_response.status(), StatusCode::OK);

        let get_response = app
            .oneshot(
                Request::delete(format!("/invoice?id={invoice_id}"))
                    .body(Empty::new())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(get_response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn invoice_ids() {
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

        let (mut app, _) = internal(State::<InMemory, MonerodMockClient>::new(
            payment_gateway,
            ServerConfig::default(),
        ));

        for _i in 0..5 {
            // `Router` implements `tower::Service<Request<Body>>` so we can
            // call it like any tower service, no need to run an HTTP server.
            let creation_response = app
                .call(
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

            dbg!(creation_response.body());
            assert_eq!(creation_response.status(), StatusCode::OK);
        }

        let response = app
            .oneshot(Request::get("/invoice/ids").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let invoice_ids: Vec<Base64InvoiceId> = serde_json::from_slice(&body).unwrap();
        dbg!(&invoice_ids);
        // Check the length of the invoice IDs vector
        assert_eq!(invoice_ids.len(), 5);
    }
}
