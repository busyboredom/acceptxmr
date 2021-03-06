use std::env;

use actix_files::Files;
use actix_session::{
    config::CookieContentSecurity, storage::CookieSessionStore, Session, SessionMiddleware,
};
use actix_web::{
    cookie, get,
    http::StatusCode,
    post,
    web::{Data, Form},
    App, HttpResponse, HttpServer, Result,
};
use handlebars::{no_escape, Handlebars};
use log::{debug, error, info};
use qrcode::{render::svg, EcLevel, QrCode};
use rand::{thread_rng, Rng};
use serde::Deserialize;
use serde_json::json;

use acceptxmr::{AcceptXmrError, InvoiceId, PaymentGateway, PaymentGatewayBuilder};

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
            .daemon_url("http://node.sethforprivacy.com:18089")
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
            // If it's been tracked for longer than an hour, remove it.
            if invoice
                .current_height()
                .saturating_sub(invoice.creation_height())
                > 30
            {
                debug!(
                    "Invoice to index {} has been tracked for > 30 blocks. Removing invoice now",
                    invoice.index()
                );
                if let Err(e) = gateway_copy.remove_invoice(invoice.id()) {
                    error!("Failed to remove invoice: {}", e);
                };
            }
        }
    });

    // Create secure session key for cookies.
    let mut key_arr = [0u8; SESSION_KEY_LEN];
    thread_rng().fill(&mut key_arr[..]);
    let session_key = cookie::Key::generate();

    // Templating setup.
    let mut handlebars = Handlebars::new();
    handlebars
        .register_templates_directory(".html", "./examples/nojs/static/templates")
        .expect("failed to register template directory");
    handlebars.register_escape_fn(no_escape);

    // Run the demo webpage.
    let shared_payment_gateway = Data::new(payment_gateway);
    let handlebars_ref = Data::new(handlebars);
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
            .app_data(handlebars_ref.clone())
            .service(start_checkout)
            .service(checkout)
            .service(Files::new("", "./examples/nojs/static").index_file("index.html"))
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
#[post("/checkout")]
async fn start_checkout(
    session: Session,
    checkout_info: Form<CheckoutInfo>,
    payment_gateway: Data<PaymentGateway>,
) -> Result<HttpResponse, actix_web::Error> {
    let invoice_id = payment_gateway
        .new_invoice(1_000_000_000, 2, 5, checkout_info.message.clone())
        .await
        .unwrap();
    session.insert("id", invoice_id)?;
    Ok(HttpResponse::TemporaryRedirect()
        .status(StatusCode::SEE_OTHER)
        .append_header(("location", "http://localhost:8080/checkout"))
        .finish())
}

// Get invoice update.
#[get("/checkout")]
async fn checkout(
    session: Session,
    payment_gateway: Data<PaymentGateway>,
    templater: Data<Handlebars<'_>>,
) -> Result<HttpResponse, actix_web::Error> {
    if let Ok(Some(invoice_id)) = session.get::<InvoiceId>("id") {
        if let Ok(Some(invoice)) = payment_gateway.get_invoice(invoice_id) {
            let mut instruction = "Send Monero to Address Below";
            let mut address = invoice.address();
            let mut qrcode = qrcode(&invoice.uri());
            if invoice.is_confirmed() {
                instruction = "Paid! Thank You";
            } else if invoice.amount_paid() >= invoice.amount_requested() {
                instruction = "Paid! Waiting for confirmations...";
            } else if invoice.expiration_in() < 3 {
                instruction = "Address Expiring Soon!";
                address = "Expiring soon...";
                qrcode = "<svg viewBox=\"0 0 100 100\" id=\"qrcode\" src=\"\"></svg>".to_string();
            }
            let data = json!({
                "instruction": instruction,
                "address": address,
                "qrcode": qrcode,
                "paid": invoice.xmr_paid(),
                "requested": invoice.xmr_requested(),
                "confirmations": invoice.confirmations().unwrap_or_default(),
                "confirmations-required": invoice.confirmations_required(),
            });
            let body = templater.render("checkout", &data).unwrap();

            // So long as the invoice did not expire while unpaid, show checkout page with updated
            // info.
            if !invoice.is_expired() || invoice.amount_paid() >= invoice.amount_requested() {
                return Ok(HttpResponse::Ok().body(body));
            }
        }
    }
    Ok(HttpResponse::TemporaryRedirect()
        .append_header(("location", "http://localhost:8080/expired.html"))
        .finish())
}

fn qrcode(uri: &str) -> String {
    let code =
        QrCode::with_error_correction_level(uri, EcLevel::M).expect("failed to build QR code");
    let image = code.render::<svg::Color>().module_dimensions(2, 2).build();
    image
}
