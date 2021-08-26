use std::env;
use std::time::{Duration, Instant};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Mutex;

use actix::{Actor, StreamHandler, ActorContext, AsyncContext};
use actix_files;
use actix_web::web::Data;
use actix_web::{get, web, App, HttpRequest, HttpResponse, HttpServer};
use actix_web_actors::ws;
use log::{debug, warn};

use acceptxmr::{BlockScanner, BlockScannerBuilder, Payment};

/// How often heartbeat pings are sent
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(4);
/// How long before lack of client response causes a timeout
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);
/// Munimunm interval for a websocket to send a payment update.
const UPDATE_INTERVAL: Duration = Duration::from_secs(1);

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    env::set_var("RUST_LOG", "debug,mio=debug,want=debug,reqwest=info");
    env_logger::init();

    // Prepare Viewkey.
    let mut viewkey_string = include_str!("../../../secrets/xmr_private_viewkey.txt").to_string();
    viewkey_string.pop(); // Remove eof charactar.

    let xmr_daemon_url = "http://busyboredom.com:18081";
    let mut block_scanner = BlockScannerBuilder::new()
        .daemon_url(xmr_daemon_url)
        .private_viewkey(&viewkey_string)
        .public_spendkey("dd4c491d53ad6b46cda01ed6cb9bac57615d9eac8d5e4dd1c0363ac8dfd420a7")
        .scan_rate(1000)
        .build();

    let current_height = block_scanner.get_current_height().await.unwrap();
    block_scanner.run(10, current_height - 10);

    let shared_block_scanner = Data::new(Mutex::new(block_scanner));

    HttpServer::new(move || {
        App::new()
            .app_data(shared_block_scanner.clone())
            .service(index)
            .service(actix_files::Files::new("/", "./static"))
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
            update_rx
        }
    }

    /// helper method that sends ping to client every HEARTBEAT_INTERVAL.
    ///
    /// also this method checks heartbeats from client
    fn hb(&self, ctx: &mut <Self as Actor>::Context) {
        ctx.run_interval(HEARTBEAT_INTERVAL, |act, ctx| {
            // check client heartbeats
            if Instant::now().duration_since(act.heartbeat) > CLIENT_TIMEOUT {
                // heartbeat timed out
                warn!("Websocket Client heartbeat failed, disconnecting!");

                // stop actor
                ctx.stop();

                // don't try to send a ping
                return;
            }

            ctx.ping(b"");
        });
    }

    fn check_update(&self, ctx: &mut <Self as Actor>::Context) {
        ctx.run_interval(UPDATE_INTERVAL, |act, ctx| {
                match act.update_rx.try_recv() {
                    // Send an update of we got one.
                    Ok(payment_update) => ctx.text(format!("{:?}", payment_update)),
                    // Otherwise, handle the error.
                    Err(e) => match e {
                        // Do nothing.
                        TryRecvError::Empty => return,
                        // Give up, something went wrong.
                        _ => {
                            // heartbeat timed out
                            warn!("Websocket failed to recieve payment update, disconnecting!");
                            // stop actor
                            ctx.stop();
                        }
                    }
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
        debug!("WebSocket message: {:?}", msg);
        match msg {
            Ok(ws::Message::Ping(msg)) => {
                self.heartbeat = Instant::now();
                ctx.pong(&msg);
            }
            Ok(ws::Message::Pong(_)) => {
                self.heartbeat = Instant::now();
            }
            Ok(ws::Message::Text(text)) => ctx.text(text),
            Ok(ws::Message::Binary(bin)) => ctx.binary(bin),
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
async fn index(req: HttpRequest, stream: web::Payload, block_scanner: web::Data<Mutex<BlockScanner>>) -> Result<HttpResponse, actix_web::Error> {
    let block_scanner = block_scanner.lock().unwrap();
    let (_address, id) = block_scanner.new_integrated_address();
    let payment = Payment::new(&id, 1, 2, 9999999);
    let receiver = block_scanner.track_payment(payment);
    let resp = ws::start(WebSocket::new(receiver), &req, stream);
    println!("{:?}", resp);
    resp
}
