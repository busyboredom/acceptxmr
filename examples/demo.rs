use std::env;
use std::path::Path;
use std::time::{Duration, Instant};

use actix::{Actor, ActorContext, AsyncContext, StreamHandler};
use actix_web::web::Data;
use actix_web::{get, web, App, HttpRequest, HttpResponse, HttpServer};
use actix_web_actors::ws;
use bytestring::ByteString;
use log::{debug, error, trace, warn};
use serde_json::json;

use acceptxmr::{AcceptXmrError, PaymentGateway, PaymentGatewayBuilder, SubIndex, Subscriber};

/// How often heartbeat pings are sent
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(4);
/// How long before lack of client response causes a timeout
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);
/// Minimum interval for a websocket to send a payment update.
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

    payment_gateway
        .run()
        .await
        .expect("failed to run payment gateway");

    // Watch for payment updates and deal with them accordingly.
    let gateway_copy = payment_gateway.clone();
    std::thread::spawn(move || {
        // Watch all payment updates by subscribing to the primary address index (0/0).
        let mut subscriber = gateway_copy.subscribe(SubIndex::new(0, 0));
        loop {
            let payment = match subscriber.recv() {
                Ok(p) => p,
                Err(AcceptXmrError::SubscriberRecv) => panic!("Blockchain scanner crashed!"),
                Err(e) => {
                    error!("Error retrieving payment update: {}", e);
                    continue;
                }
            };
            // If it's confirmed or expired, we probably shouldn't bother tracking it anymore.
            if (payment.is_confirmed() && payment.creation_height() < payment.current_height())
                || payment.is_expired()
            {
                debug!(
                    "Payment to index {} is either confirmed or expired. Removing payment now",
                    payment.index()
                );
                if let Err(e) = gateway_copy.remove_payment(payment.index()) {
                    error!("Failed to remove fully confirmed payment: {}", e);
                };
            }
        }
    });

    let shared_payment_gateway = Data::new(payment_gateway);
    HttpServer::new(move || {
        App::new()
            .app_data(shared_payment_gateway.clone())
            .service(websocket)
            .service(actix_files::Files::new("", "./examples/static").index_file("index.html"))
    })
    .bind("0.0.0.0:8080")?
    .run()
    .await
}

/// Define websocket HTTP actor
struct WebSocket {
    heartbeat: Instant,
    payment_subscriber: Subscriber,
}

impl WebSocket {
    fn new(payment_subscriber: Subscriber) -> Self {
        Self {
            heartbeat: Instant::now(),
            payment_subscriber,
        }
    }

    /// Check subscriber for payment update, and send result to user if applicable.
    fn check_update(&self, ctx: &mut <Self as Actor>::Context) {
        ctx.run_interval(UPDATE_INTERVAL, |act, ctx| {
            match act.payment_subscriber.next() {
                // Send an update of we got one.
                Some(Ok(payment_update)) => {
                    // Send the update to the user.
                    ctx.text(ByteString::from(
                        json!(
                            {
                                "address": payment_update.address(),
                                "amount_paid": payment_update.amount_paid(),
                                "amount_requested": payment_update.amount_requested(),
                                "confirmations": payment_update.confirmations(),
                                "expiration_in": payment_update.expiration_in(),
                            }
                        )
                        .to_string(),
                    ));
                    // If the payment is confirmed or expired, stop checking for updates.
                    if payment_update.is_confirmed() {
                        ctx.close(Some(ws::CloseReason::from((
                            ws::CloseCode::Normal,
                            "Payment Complete",
                        ))));
                        ctx.stop();
                    } else if payment_update.is_expired() {
                        ctx.close(Some(ws::CloseReason::from((
                            ws::CloseCode::Normal,
                            "Payment Expired",
                        ))));
                        ctx.stop();
                    }
                }
                // Otherwise, handle the error.
                Some(Err(e)) => {
                    error!("Failed to receive payment update: {}", e);
                }
                // Or do nothing if nothing was received.
                None => {}
            }
        });
    }

    /// Helper method that sends ping to client every HEARTBEAT_INTERVAL.
    fn hb(&self, ctx: &mut <Self as Actor>::Context) {
        ctx.run_interval(HEARTBEAT_INTERVAL, |act, ctx| {
            // Check client heartbeats.
            if Instant::now().duration_since(act.heartbeat) > CLIENT_TIMEOUT {
                warn!("Websocket Client heartbeat failed, disconnecting!");
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
        trace!("WebSocket message: {:?}", msg);
        match msg {
            Ok(ws::Message::Ping(msg)) => {
                self.heartbeat = Instant::now();
                ctx.pong(&msg);
            }
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

/// WebSocket handler.
#[get("/ws/")]
async fn websocket(
    req: HttpRequest,
    stream: web::Payload,
    payment_gateway: web::Data<PaymentGateway>,
) -> Result<HttpResponse, actix_web::Error> {
    // TODO: Use cookies to determine if a purchase is already pending, and avoid creating a new one.
    let subscriber = payment_gateway.new_payment(100, 2, 3).await.unwrap();
    ws::start(WebSocket::new(subscriber), &req, stream)
}
