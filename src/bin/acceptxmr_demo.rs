use std::env;
use std::path::Path;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use actix::{Actor, ActorContext, AsyncContext, StreamHandler};
use actix_web::web::Data;
use actix_web::{get, web, App, HttpRequest, HttpResponse, HttpServer};
use actix_web_actors::ws;
use bytestring::ByteString;
use log::{debug, trace, warn};

use acceptxmr::{Payment, PaymentProcessor, PaymentProcessorBuilder};

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
        "trace,mio=debug,want=debug,reqwest=info,sled=debug",
    );
    env_logger::init();

    // Prepare Viewkey.
    let private_viewkey_path = Path::new("../secrets/xmr_private_viewkey.txt");
    let mut viewkey_string = std::fs::read_to_string(private_viewkey_path)
        .expect("Failed to read private viewkey from file, are you sure it exists?");
    viewkey_string = viewkey_string // Remove line endind in a cross-platform friendly way.
        .strip_suffix("\r\n")
        .or_else(|| viewkey_string.strip_suffix('\n'))
        .unwrap_or(&viewkey_string)
        .to_string();

    let xmr_daemon_url = "http://busyboredom.com:18081";
    let mut payment_processor = PaymentProcessorBuilder::new()
        .daemon_url(xmr_daemon_url)
        .private_viewkey(&viewkey_string)
        .public_spendkey("dd4c491d53ad6b46cda01ed6cb9bac57615d9eac8d5e4dd1c0363ac8dfd420a7")
        .scan_rate(1000)
        .build();

    let current_height = payment_processor.get_current_height().await.unwrap();
    payment_processor.run(10, current_height - 10);

    let shared_payment_processor = Data::new(Mutex::new(payment_processor));

    HttpServer::new(move || {
        App::new()
            .app_data(shared_payment_processor.clone())
            .service(websocket)
            .service(actix_files::Files::new("", "./static").index_file("index.html"))
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}

/// Define HTTP actor
struct WebSocket {
    heartbeat: Instant,
    update_rx: Receiver<Payment>,
}

impl WebSocket {
    fn new(update_rx: Receiver<Payment>) -> Self {
        Self {
            heartbeat: Instant::now(),
            update_rx,
        }
    }

    /// helper method that sends ping to client every HEARTBEAT_INTERVAL.
    ///
    /// also this method checks heartbeats from client
    fn hb(&self, ctx: &mut <Self as Actor>::Context) {
        ctx.run_interval(HEARTBEAT_INTERVAL, |act, ctx| {
            // check client heartbeats.
            if Instant::now().duration_since(act.heartbeat) > CLIENT_TIMEOUT {
                // heartbeat timed out.
                warn!("Websocket Client heartbeat failed, disconnecting!");

                // stop actor.
                ctx.stop();

                // don't try to send a ping.
                return;
            }

            ctx.ping(b"");
        });
    }

    fn check_update(&self, ctx: &mut <Self as Actor>::Context) {
        ctx.run_interval(UPDATE_INTERVAL, |act, ctx| {
            match act.update_rx.try_recv() {
                // Send an update of we got one.
                Ok(payment_update) => {
                    // Log the update first.
                    let confirmations = match payment_update.paid_at {
                        Some(height) => payment_update.current_block.saturating_sub(height).to_string(),
                        None => "N/A".to_string(),
                    };
                    debug!(
                        "Payment update for subaddress index {}:\n\tPaid: {}/{}\n\tConfirmations: {}\n\tCurrent block: {}", 
                        payment_update.index,
                        monero::Amount::from_pico(payment_update.paid_amount).as_xmr(),
                        monero::Amount::from_pico(payment_update.expected_amount).as_xmr(),
                        confirmations,
                        payment_update.current_block,
                    );

                    // Serialize the payment object.
                    let mut payment_json = serde_json::to_value(&payment_update)
                        .expect("Failed to serialize payment update.");
                    // User doesn't need the subaddress index, so remove it.
                    payment_json.as_object_mut().unwrap().remove("index");
                    // Convert to string.
                    let payment_string = payment_json.to_string();

                    // Send the update to the user.
                    ctx.text(ByteString::from(payment_string));

                    // if the payment is confirmed or expired, stop checking for updates.
                    if payment_update.is_confirmed() {
                        debug!("Payment to index {} fully confirmed!", payment_update.index);
                        ctx.close(Some(ws::CloseReason::from((ws::CloseCode::Normal, "Payment Complete"))));
                        ctx.stop();
                    } else if payment_update.is_expired() {
                        debug!("Payment to index {} expired before full confimration.", payment_update.index);
                        ctx.close(Some(ws::CloseReason::from((ws::CloseCode::Normal, "Payment Expired"))));
                        ctx.stop();
                    }
                }
                // Otherwise, handle the error.
                Err(e) => match e {
                    // Do nothing.
                    TryRecvError::Empty => {},
                    // Give up, something went wrong.
                    _ => {
                        warn!("Websocket failed to recieve payment update, disconnecting!");
                        ctx.stop();
                    }
                },
            }
        });
    }
}

impl Actor for WebSocket {
    type Context = ws::WebsocketContext<Self>;

    /// Method is called on actor start. We start the heartbeat process here.
    fn started(&mut self, ctx: &mut Self::Context) {
        self.hb(ctx);
        self.check_update(ctx);
    }
}

/// Handler for ws::Message message
impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for WebSocket {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        // process websocket messages
        trace!("WebSocket message: {:?}", msg);
        match msg {
            Ok(ws::Message::Ping(msg)) => {
                self.heartbeat = Instant::now();
                ctx.pong(&msg);
            }
            Ok(ws::Message::Pong(_)) => {
                self.heartbeat = Instant::now();
            }
            Ok(ws::Message::Text(text)) => debug!("Recieved from websocket: {}", text),
            Ok(ws::Message::Binary(bin)) => debug!("Recieved from websocket: {:?}", bin),
            Ok(ws::Message::Close(reason)) => {
                ctx.close(reason);
                ctx.stop();
            }
            _ => ctx.stop(),
        }
    }
}

/// WebSocket handler.
#[get("/ws/")]
async fn websocket(
    req: HttpRequest,
    stream: web::Payload,
    payment_processor: web::Data<Mutex<PaymentProcessor>>,
) -> Result<HttpResponse, actix_web::Error> {
    // TODO: Use cookies to determine if a purchase is already pending, and avoid creating a new one.
    let payment_processor = payment_processor.lock().unwrap();
    let (address, subindex) = payment_processor.new_subaddress();
    let current_block = payment_processor
        .get_current_height()
        .await
        .expect("Failed to get current height");
    let payment = Payment::new(&address, subindex, current_block, 1, 2, current_block + 3);
    let receiver = payment_processor.track_payment(payment);

    ws::start(WebSocket::new(receiver), &req, stream)
}
