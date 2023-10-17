use acceptxmr::{storage::stores::Sqlite, PaymentGateway};
use actix_session::Session;
use actix_web::{
    http::header::{CacheControl, CacheDirective},
    post, web, HttpResponse,
};
use log::debug;
use serde::Deserialize;

pub fn internal(cfg: &mut web::ServiceConfig) {
    cfg.service(checkout);
}

#[derive(Deserialize)]
struct CheckoutInfo {
    message: String,
}

/// Create new invoice and place cookie.
#[allow(clippy::unused_async)]
#[post("/checkout")]
async fn checkout(
    session: Session,
    checkout_info: web::Json<CheckoutInfo>,
    payment_gateway: web::Data<PaymentGateway<Sqlite>>,
) -> Result<HttpResponse, actix_web::Error> {
    let invoice_id = payment_gateway
        .new_invoice(1_000_000_000, 2, 5, checkout_info.message.clone())
        .unwrap();
    session.insert("id", invoice_id)?;
    debug!("Checkout out successfully. Invoice ID: {}", invoice_id);
    Ok(HttpResponse::Ok()
        .append_header(CacheControl(vec![CacheDirective::NoStore]))
        .finish())
}
