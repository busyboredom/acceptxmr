use acceptxmr::{storage::stores::Sqlite, InvoiceId, PaymentGateway};
use actix_session::Session;
use actix_web::{
    get,
    http::header::{CacheControl, CacheDirective},
    web, HttpRequest, HttpResponse,
};
use actix_web_actors::ws;
use serde_json::json;

use crate::server::WebSocket;

pub fn external(service_config: &mut web::ServiceConfig) {
    service_config.service(update).service(websocket);
}

// Get invoice update without waiting for websocket.
#[allow(clippy::unused_async)]
#[get("/update")]
async fn update(
    session: Session,
    payment_gateway: web::Data<PaymentGateway<Sqlite>>,
) -> Result<HttpResponse, actix_web::Error> {
    if let Ok(Some(invoice_id)) = session.get::<InvoiceId>("id") {
        if let Ok(Some(invoice)) = payment_gateway.get_invoice(invoice_id) {
            return Ok(HttpResponse::Ok()
                .append_header(CacheControl(vec![CacheDirective::NoStore]))
                .json(json!(
                    {
                        "address": invoice.address(),
                        "amount_paid": invoice.amount_paid(),
                        "amount_requested": invoice.amount_requested(),
                        "uri": invoice.uri(),
                        "confirmations": invoice.confirmations(),
                        "confirmations_required": invoice.confirmations_required(),
                        "expiration_in": invoice.expiration_in(),
                    }
                )));
        };
    }
    Ok(HttpResponse::Gone()
        .append_header(CacheControl(vec![CacheDirective::NoStore]))
        .finish())
}

/// WebSocket rout.
#[allow(clippy::unused_async)]
#[get("/ws/")]
async fn websocket(
    session: Session,
    req: HttpRequest,
    stream: web::Payload,
    payment_gateway: web::Data<PaymentGateway<Sqlite>>,
) -> Result<HttpResponse, actix_web::Error> {
    let Ok(Some(invoice_id)) = session.get::<InvoiceId>("id") else {
        return Ok(HttpResponse::NotFound()
            .append_header(CacheControl(vec![CacheDirective::NoStore]))
            .finish());
    };
    let Some(subscriber) = payment_gateway.subscribe(invoice_id) else {
        return Ok(HttpResponse::NotFound()
            .append_header(CacheControl(vec![CacheDirective::NoStore]))
            .finish());
    };
    let websocket = WebSocket::new(subscriber);
    ws::start(websocket, &req, stream)
}
