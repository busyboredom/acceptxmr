use std::{
    env,
    time::{Duration, Instant},
};

use actix::{Actor, ActorContext, AsyncContext, StreamHandler};
use actix_files::Files;
use actix_session::{
    storage::CookieSessionStore, CookieContentSecurity, Session, SessionMiddleware,
};
use actix_web::{cookie, get, post, web, web::Data, App, HttpRequest, HttpResponse, HttpServer};
use actix_web_actors::ws;
use bytestring::ByteString;
use log::{debug, error, info, warn};
use rand::{thread_rng, Rng};
use serde::Deserialize;
use serde_json::json;

use acceptxmr::{
    AcceptXmrError, InvoiceId, PaymentGateway, PaymentGatewayBuilder, Subscriber, SubscriberError,
};

/// Time before lack of client response causes a timeout.
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);
/// Time between sending heartbeat pings.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);
/// Length of secure session key for cookies.
const SESSION_KEY_LEN: usize = 64;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    env::set_var(
        "RUST_LOG",
        "debug,mio=debug,want=debug,sled=debug,hyper=info,tracing=debug,actix_http=debug",
    );
    env_logger::init();

    // The private view key should be stored securely outside of the git repository. It is hardcoded
    // here for demonstration purposes only.
    let private_view_key = "ad2093a5705b9f33e6f0f0c1bc1f5f639c756cdfc168c8f2ac6127ccbdab3a03";

    // No need to keep the primary address secret.
    let primary_address = "4613YiHLM6JMH4zejMB2zJY5TwQCxL8p65ufw8kBP5yxX9itmuGLqp1dS4tkVoTxjyH3aYhYNrtGHbQzJQP5bFus3KHVdmf";

    let payment_gateway =
        PaymentGatewayBuilder::new(private_view_key.to_string(), primary_address.to_string())
            .daemon_url("http://busyboredom.com:18089")
            .build()
            .expect("failed to build payment gateway");
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

    // Create secure session key for cookies.
    let mut key_arr = [0u8; SESSION_KEY_LEN];
    thread_rng().fill(&mut key_arr[..]);
    let session_key = cookie::Key::generate();

    // Run the demo webpage.
    let shared_payment_gateway = Data::new(payment_gateway);
    HttpServer::new(move || {
        App::new()
            .wrap(
                SessionMiddleware::builder(CookieSessionStore::default(), session_key.clone())
                    .cookie_name("acceptxmr_session".to_string())
                    .cookie_secure(false)
                    .cookie_content_security(CookieContentSecurity::Private)
                    .build(),
            )
            .app_data(shared_payment_gateway.clone())
            .service(index)
            .service(check_out)
            .service(websocket)
            .service(Files::new("", "./examples/static").index_file("index.html"))
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
        .new_invoice(1_000_000_000, 2, 3, checkout_info.message.clone())
        .await
        .unwrap();
    session.insert("id", invoice_id)?;
    Ok("Success")
}

// Get invoice update without waiting for websocket.
#[get("/invoice")]
async fn index(
    session: Session,
    payment_gateway: web::Data<PaymentGateway>,
) -> Result<HttpResponse, actix_web::Error> {
    if let Ok(Some(invoice_id)) = session.get::<InvoiceId>("id") {
        if let Ok(Some(invoice)) = payment_gateway.get_invoice(invoice_id) {
            return Ok(HttpResponse::Ok().json(json!(
                {
                    "address": invoice.address(),
                    "amount_paid": invoice.amount_paid(),
                    "amount_requested": invoice.amount_requested(),
                    "payment_request": invoice.payment_request(),
                    "confirmations": invoice.confirmations(),
                    "confirmations_required": invoice.confirmations_required(),
                    "expiration_in": invoice.expiration_in(),
                }
            )));
        };
    }
    Ok(HttpResponse::Gone().finish())
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
                            "payment_request": invoice_update.payment_request(),
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
    /// sending a heartbeat ping as well.
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
