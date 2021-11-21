use std::env;
use std::path::Path;
use std::time::{Duration, Instant};

use actix::{Actor, ActorContext, AsyncContext, StreamHandler};
use actix_session::{CookieSession, Session};
use actix_web::web::Data;
use actix_web::{get, post, web, App, HttpRequest, HttpResponse, HttpServer};
use actix_web_actors::ws;
use bytestring::ByteString;
use log::{debug, error, info, warn};
use serde::Deserialize;
use serde_json::json;

use acceptxmr::{
    AcceptXmrError, InvoiceId, PaymentGateway, PaymentGatewayBuilder, Subscriber, SubscriberError,
};

/// Time before lack of client response causes a timeout.
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);
/// Time between sending heartbeat pings.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    env::set_var(
        "RUST_LOG",
        "trace,mio=debug,want=debug,reqwest=info,sled=debug,hyper=info,tracing=debug",
    );
    env_logger::init();

    // Read view key from file outside of git repository.
    let private_view_key =
        std::fs::read_to_string(Path::new("../secrets/xmr_private_view_key.txt"))
            .expect("Failed to read private view key from file, are you sure it exists?")
            .trim() // Remove line ending.
            .to_owned();

    // No need to keep the public spend key secret.
    let public_spend_key = "dd4c491d53ad6b46cda01ed6cb9bac57615d9eac8d5e4dd1c0363ac8dfd420a7";

    let payment_gateway = PaymentGatewayBuilder::new(&private_view_key, public_spend_key)
        .daemon_url("http://busyboredom.com:18081")
        .build();
    info!("Payment gateway created.");

    payment_gateway
        .run()
        .await
        .expect("failed to run payment gateway");
    info!("Payment gateway running.");

    // Watch for invoice updates and deal with them accordingly.
    let gateway_copy = payment_gateway.clone();
    std::thread::spawn(move || {
        // Watch all invoice updates.
        let mut subscriber = gateway_copy.subscribe_all();
        loop {
            let invoice = match subscriber.recv() {
                Ok(p) => p,
                Err(AcceptXmrError::Subscriber(_)) => panic!("Blockchain scanner crashed!"),
                Err(e) => {
                    error!("Error retrieving invoice update: {}", e);
                    continue;
                }
            };
            // If it's confirmed or expired, we probably shouldn't bother tracking it anymore.
            if (invoice.is_confirmed() && invoice.creation_height() < invoice.current_height())
                || invoice.is_expired()
            {
                debug!(
                    "Invoice to index {} is either confirmed or expired. Removing invoice now",
                    invoice.index()
                );
                if let Err(e) = gateway_copy.remove_invoice(invoice.id()) {
                    error!("Failed to remove fully confirmed invoice: {}", e);
                };
            }
        }
    });

    // Run the demo webpage.
    let shared_payment_gateway = Data::new(payment_gateway);
    HttpServer::new(move || {
        App::new()
            .wrap(
                CookieSession::private(&[0; 32])
                    .domain("localhost")
                    .name("acceptxmr_session"),
            )
            .app_data(shared_payment_gateway.clone())
            .service(check_out)
            .service(websocket)
            .service(actix_files::Files::new("", "./examples/static").index_file("index.html"))
    })
    .bind("0.0.0.0:8080")?
    .run()
    .await
}

#[derive(Deserialize)]
struct CheckoutInfo {
    message: String,
}

/// Create new invoice and place cookie.
#[post("/check_out")]
async fn check_out(
    session: Session,
    checkout_info: web::Json<CheckoutInfo>,
    payment_gateway: web::Data<PaymentGateway>,
) -> Result<&'static str, actix_web::Error> {
    let invoice_id = payment_gateway
        .new_invoice(100, 2, 3, &checkout_info.message)
        .await
        .unwrap();
    session.insert("id", invoice_id)?;
    Ok("Success")
}

/// WebSocket rout.
#[get("/ws/")]
async fn websocket(
    session: Session,
    req: HttpRequest,
    stream: web::Payload,
    payment_gateway: web::Data<PaymentGateway>,
) -> Result<HttpResponse, actix_web::Error> {
    let invoice_id = match session.get::<InvoiceId>("id") {
        Ok(Some(i)) => i,
        _ => return Ok(HttpResponse::NotFound().finish()),
    };
    let subscriber = match payment_gateway.subscribe(invoice_id) {
        Ok(Some(s)) => s,
        _ => return Ok(HttpResponse::NotFound().finish()),
    };
    ws::start(WebSocket::new(subscriber), &req, stream)
}

/// Define websocket HTTP actor
struct WebSocket {
    last_check: Instant,
    client_replied: bool,
    invoice_subscriber: Subscriber,
}

impl WebSocket {
    fn new(invoice_subscriber: Subscriber) -> Self {
        Self {
            last_check: Instant::now(),
            client_replied: true,
            invoice_subscriber,
        }
    }

    /// Check subscriber for invoice update, and send result to user if applicable.
    fn try_update(&mut self, ctx: &mut <Self as Actor>::Context) {
        match self.invoice_subscriber.recv_timeout(HEARTBEAT_INTERVAL) {
            // Send an update of we got one.
            Ok(invoice_update) => {
                // Send the update to the user.
                ctx.text(ByteString::from(
                    json!(
                        {
                            "address": invoice_update.address(),
                            "amount_paid": invoice_update.amount_paid(),
                            "amount_requested": invoice_update.amount_requested(),
                            "confirmations": invoice_update.confirmations(),
                            "confirmations_required": invoice_update.confirmations_required(),
                            "expiration_in": invoice_update.expiration_in(),
                        }
                    )
                    .to_string(),
                ));
                // If the invoice is confirmed or expired, stop checking for updates.
                if invoice_update.is_confirmed() {
                    ctx.close(Some(ws::CloseReason::from((
                        ws::CloseCode::Normal,
                        "Invoice Complete",
                    ))));
                    ctx.stop();
                } else if invoice_update.is_expired() {
                    ctx.close(Some(ws::CloseReason::from((
                        ws::CloseCode::Normal,
                        "Invoice Expired",
                    ))));
                    ctx.stop();
                }
            }
            // Do nothing if there was no update.
            Err(AcceptXmrError::Subscriber(SubscriberError::RecvTimeout(
                std::sync::mpsc::RecvTimeoutError::Timeout,
            ))) => {}
            // Otherwise, handle the error.
            Err(e) => {
                error!("Failed to receive invoice update: {}", e);
                ctx.stop();
            }
        }
    }
}

impl Actor for WebSocket {
    type Context = ws::WebsocketContext<Self>;
    /// This method is called on actor start. We start waiting for updates here, periodically
    /// stopping to sent a heartbeat ping.
    fn started(&mut self, ctx: &mut Self::Context) {
        ctx.run_interval(HEARTBEAT_INTERVAL, |act, ctx| {
            // Wait for and then send an update.
            if act.client_replied {
                act.try_update(ctx);
                ctx.ping(b"");
                act.client_replied = false;
                act.last_check = Instant::now();
            // Check heartbeat.
            } else if Instant::now().duration_since(act.last_check) > CLIENT_TIMEOUT {
                warn!("Websocket heartbeat failed. Closing websocket.");
                ctx.stop();
            }
        });
    }
}

/// Handle incoming websocket messages.
impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for WebSocket {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match msg {
            Ok(ws::Message::Pong(_)) => {
                self.client_replied = true;
            }
            Ok(ws::Message::Close(reason)) => {
                match &reason {
                    Some(r) => debug!("Websocket client closing: {:#?}", r.description),
                    None => debug!("Websocket client closing"),
                }
                ctx.close(reason);
                ctx.stop();
            }
            Ok(m) => debug!("Received unexpected message from websocket client: {:?}", m),
            Err(e) => warn!("Received error from websocket client: {:?}", e),
        }
    }
}
