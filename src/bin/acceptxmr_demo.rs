use std::env;
use std::time::{Duration, Instant};
use std::sync::mpsc::Receiver;

use actix::{Actor, StreamHandler, ActorContext, AsyncContext};
use actix_files;
use actix_web::{get, web, App, HttpRequest, HttpResponse, HttpServer};
use actix_web_actors::ws;
use log::{debug, info};
use qrcode::render::string::Element;
use qrcode::render::svg;
use qrcode::QrCode;
use tokio::fs;
use bytestring::ByteString;

use acceptxmr::{BlockScannerBuilder, Payment};

/// How often heartbeat pings are sent
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(4);
/// How long before lack of client response causes a timeout
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    env::set_var("RUST_LOG", "debug,mio=debug,want=debug,reqwest=info");
    env_logger::init();

    // Prepare Viewkey.
    let mut viewkey_string = include_str!("../../../secrets/xmr_private_viewkey.txt").to_string();
    viewkey_string.pop();

    let xmr_daemon_url = "http://busyboredom.com:18081";
    let mut block_scanner = BlockScannerBuilder::new()
        .daemon_url(xmr_daemon_url)
        .private_viewkey(&viewkey_string)
        .public_spendkey("dd4c491d53ad6b46cda01ed6cb9bac57615d9eac8d5e4dd1c0363ac8dfd420a7")
        .scan_rate(1000)
        .build();

    // Get a new integrated address, and the payment ID contained in it.
    let (address, payment_id) = block_scanner.new_integrated_address();
    info!("Payment ID generated: {}", payment_id);

    // Render a QR code for the new address.
    let qr = QrCode::new(address).unwrap();
    let image = qr.render::<svg::Color>().module_dimensions(1, 1).build();

    // Save the QR code image.
    fs::write("static/qrcode.svg", image)
        .await
        .expect("Unable to write QR Code image to file");

    let current_height = block_scanner.get_current_height().await.unwrap();
    block_scanner.run(10, current_height - 10);

    let payment = Payment::new(&payment_id, 1, 1, 99999999);
    let payment_updates = block_scanner.track_payment(payment);

    HttpServer::new(|| {
        App::new()
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
                println!("Websocket Client heartbeat failed, disconnecting!");

                // stop actor
                ctx.stop();

                // don't try to send a ping
                return;
            }

            ctx.ping(b"");
        });
    }

    fn send_update(&self, ctx: &mut <Self as Actor>::Context) {
        loop {
            let payment_update = self.update_rx.recv();
            ctx.text(format!("{:?}", payment_update))
        }
    }
}

impl Actor for WebSocket {
    type Context = ws::WebsocketContext<Self>;

    /// Method is called on actor start. We start the heartbeat process here.
    fn started(&mut self, ctx: &mut Self::Context) {
        self.hb(ctx);
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
async fn index(req: HttpRequest, stream: web::Payload) -> Result<HttpResponse, actix_web::Error> {
    let resp = ws::start(WebSocket::new(), &req, stream);
    println!("{:?}", resp);
    resp
}
