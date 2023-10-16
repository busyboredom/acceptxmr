use std::{
    future::Future,
    pin::Pin,
    task::Poll,
    time::{Duration, Instant},
};

use acceptxmr::{Invoice, Subscriber};
use actix::{prelude::Stream, Actor, ActorContext, AsyncContext, StreamHandler};
use actix_web_actors::ws;
use bytestring::ByteString;
use log::{debug, warn};
use serde_json::json;

/// Time before lack of client response causes a timeout.
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);
/// Time between sending heartbeat pings.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);

/// Define websocket HTTP actor
pub struct WebSocket {
    last_heartbeat: Instant,
    invoice_subscriber: Option<Subscriber>,
}

impl WebSocket {
    pub fn new(invoice_subscriber: Subscriber) -> Self {
        Self {
            last_heartbeat: Instant::now(),
            invoice_subscriber: Some(invoice_subscriber),
        }
    }

    /// Sends ping to client every `HEARTBEAT_INTERVAL` and checks for responses
    /// from client
    fn heartbeat(ctx: &mut <Self as Actor>::Context) {
        ctx.run_interval(HEARTBEAT_INTERVAL, |act, ctx| {
            // check client heartbeats
            if Instant::now().duration_since(act.last_heartbeat) > CLIENT_TIMEOUT {
                // heartbeat timed out
                warn!("Websocket Client heartbeat failed, disconnecting!");
                ctx.stop();
                return;
            }
            ctx.ping(b"");
        });
    }
}

impl Actor for WebSocket {
    type Context = ws::WebsocketContext<Self>;

    /// This method is called on actor start. We add the invoice subscriber as a
    /// stream here, and start heartbeat checks as well.
    fn started(&mut self, ctx: &mut Self::Context) {
        if let Some(subscriber) = self.invoice_subscriber.take() {
            <WebSocket as StreamHandler<Invoice>>::add_stream(InvoiceStream(subscriber), ctx);
        }
        Self::heartbeat(ctx);
    }
}

/// Handle incoming websocket messages.
impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for WebSocket {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match msg {
            Ok(ws::Message::Pong(_)) => {
                self.last_heartbeat = Instant::now();
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

/// Handle incoming invoice updates.
impl StreamHandler<Invoice> for WebSocket {
    fn handle(&mut self, invoice_update: Invoice, ctx: &mut Self::Context) {
        // Send the update to the user.
        ctx.text(ByteString::from(
            json!(
                {
                    "address": invoice_update.address(),
                    "amount_paid": invoice_update.amount_paid(),
                    "amount_requested": invoice_update.amount_requested(),
                    "uri": invoice_update.uri(),
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
}

// Wrapping `Subscriber` and implementing `Stream` on the wrapper allows us to
// use it as an efficient asynchronous stream for the Actix websocket.
struct InvoiceStream(Subscriber);

impl Stream for InvoiceStream {
    type Item = Invoice;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.0).poll(cx)
    }
}
