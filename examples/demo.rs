use std::env;
use std::path::Path;
use std::sync::mpsc::TryRecvError;
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

use acceptxmr::subscriber::{Subscriber, SubscriberError};
use acceptxmr::{AcceptXmrError, PaymentGateway, PaymentGatewayBuilder, SubIndex};

/// How often heartbeat pings are sent
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(4);
/// How long before lack of client response causes a timeout
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);
/// Minimum interval for a websocket to send an invoice update.
const UPDATE_INTERVAL: Duration = Duration::from_millis(100);

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
        .scan_interval(Duration::from_millis(1000))
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
        // Watch all invoice updates by subscribing to the primary address index (0/0).
        let mut subscriber = gateway_copy.subscribe(SubIndex::new(0, 0));
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
                if let Err(e) = gateway_copy.remove_invoice(invoice.index()) {
                    error!("Failed to remove fully confirmed invoice: {}", e);
                };
            }
        }
    });

    let shared_payment_gateway = Data::new(payment_gateway);
    HttpServer::new(move || {
        App::new()
            .wrap(
                CookieSession::signed(&[0; 32])
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
    info!("Donor message: {}", checkout_info.message);
    let (sub_index, _height) = payment_gateway.new_invoice(100, 2, 3).await.unwrap();
    session.insert("index", sub_index)?;
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
    let sub_index = match session.get::<SubIndex>("index") {
        Ok(Some(i)) => i,
        _ => return Ok(HttpResponse::NotFound().finish()),
    };
    let subscriber = payment_gateway.subscribe(sub_index);
    ws::start(WebSocket::new(subscriber), &req, stream)
}

/// Define websocket HTTP actor
struct WebSocket {
    heartbeat: Instant,
    invoice_subscriber: Subscriber,
}

impl WebSocket {
    fn new(invoice_subscriber: Subscriber) -> Self {
        Self {
            heartbeat: Instant::now(),
            invoice_subscriber,
        }
    }

    /// Check subscriber for invoice update, and send result to user if applicable.
    fn check_update(&self, ctx: &mut <Self as Actor>::Context) {
        ctx.run_interval(UPDATE_INTERVAL, |act, ctx| {
            match act.invoice_subscriber.try_recv() {
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
                // Do nothing if nothing was received.
                Err(AcceptXmrError::Subscriber(SubscriberError::TryRecv(TryRecvError::Empty))) => {}
                // Otherwise, handle the error.
                Err(e) => {
                    error!("Failed to receive invoice update: {}", e);
                    ctx.stop();
                }
            }
        });
    }

    /// Helper method that sends ping to client every HEARTBEAT_INTERVAL.
    fn hb(&self, ctx: &mut <Self as Actor>::Context) {
        ctx.run_interval(HEARTBEAT_INTERVAL, |act, ctx| {
            // Check client heartbeats.
            if Instant::now().duration_since(act.heartbeat) > CLIENT_TIMEOUT {
                warn!("Websocket heartbeat failed. Closing websocket.");
                ctx.stop();
            } else {
                ctx.ping(b"");
            }
        });
    }
}

impl Actor for WebSocket {
    type Context = ws::WebsocketContext<Self>;
    /// This method is called on actor start. We start the heartbeat process here, and also start
    /// checking for updates.
    fn started(&mut self, ctx: &mut Self::Context) {
        self.hb(ctx);
        self.check_update(ctx);
    }
}

/// Handle incoming websocket messages.
impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for WebSocket {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match msg {
            Ok(ws::Message::Pong(_)) => {
                self.heartbeat = Instant::now();
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
