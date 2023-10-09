use std::str::FromStr;

use axum::{
    extract::State as AxumState, http::HeaderValue, response::IntoResponse, routing::post, Json,
    Router,
};
use hyper::http::{header::CACHE_CONTROL, Uri};
use log::debug;
use serde::Deserialize;
use utoipa::{IntoParams, OpenApi, ToSchema};
use utoipa_swagger_ui::SwaggerUi;

use crate::server::{
    api::{ApiError, InvoiceDescription, InvoiceIdPayload},
    State,
};

#[derive(OpenApi)]
#[openapi(
    paths(
        new_invoice,
    ),
    components(
        schemas(InvoiceIdPayload, NewInvoiceParams)
    ),
    tags(
        (name = "Internal API", description = "AcceptXMR's non-user-facing API")
    )
)]
struct ApiDoc;

pub(crate) fn internal(state: State) -> Router {
    Router::new()
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi()))
        .route("/invoice", post(new_invoice))
        .with_state(state)
}

#[derive(Deserialize, ToSchema, IntoParams)]
struct NewInvoiceParams {
    piconeros_due: u64,
    confirmations_required: u64,
    expiration_in: u64,
    message: String,
    callback: Option<String>,
}

/// Create a new invoice.
///
/// Create a new invoice with the provided details. Returns the ID of the new
/// invoice.
#[utoipa::path(
    post,
    path = "/invoice",
    params(
        NewInvoiceParams
    ),
    responses(
        (status = 200, description = "Created a new invoice", body = InvoiceId)
    )
)]
async fn new_invoice(
    AxumState(state): AxumState<State>,
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
                message: payload.message.clone(),
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
