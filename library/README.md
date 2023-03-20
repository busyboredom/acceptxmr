[![BuildStatus](https://github.com/busyboredom/acceptxmr/workflows/CI/badge.svg)](https://img.shields.io/github/actions/workflow/status/busyboredom/acceptxmr/ci.yml?branch=main)
[![Crates.io](https://img.shields.io/crates/v/acceptxmr.svg)](https://crates.io/crates/acceptxmr)
[![Documentation](https://docs.rs/acceptxmr/badge.svg)](https://docs.rs/acceptxmr)
[![MSRV](https://img.shields.io/badge/MSRV-1.65.0-blue)](https://blog.rust-lang.org/2022/11/03/Rust-1.65.0.html)

# `AcceptXMR`: Accept Monero in Your Application
`AcceptXMR` is a library for building payment gateways. 

For a batteries-included gateway, please see
[`AcceptXMR-Server`](../server/).

## Getting Started

To use `AcceptXMR` in your rust project, first add it to your `Cargo.toml`. For
example if you intend to use the `Sqlite` storage backend and need `serde`
support, you should add this to your `Cargo.toml`:
```toml
[dependencies]
acceptxmr = { version = "0.12", features = ["serde", "sqlite"] }
```
You can then create and run a `PaymentGateway`:
```rust
use acceptxmr::{PaymentGateway, storage::stores::Sqlite};
use std::time::Duration;

let private_view_key = 
  "ad2093a5705b9f33e6f0f0c1bc1f5f639c756cdfc168c8f2ac6127ccbdab3a03";
let primary_address = 
  "4613YiHLM6JMH4zejMB2zJY5TwQCxL8p65ufw8kBP5yxX9itmuGLqp1dS4tkVoTxjyH3aYhYNrtGHbQzJQP5bFus3KHVdmf";

let store = Sqlite::new("AcceptXMR_DB", "invoices")?;

let payment_gateway = PaymentGateway::builder(
  private_view_key.to_string(),
  primary_address.to_string(),
  store
)
.daemon_url("https://node.example.com") // Specify a node.
.scan_interval(Duration::from_millis(500)) // Scan for updates every 500 ms.
.build()?;

payment_gateway.run()?;
```
Finally, you can create invoices and subscribe to them so you know when they get
paid:
```rust
// Oh hey, a customer is checking out!
let invoice_id = payment_gateway.new_invoice(
  100 * 10 ** 9,                    // We'll charge 100 millineros,
  0,                                // require 0 confirmations,
  10,                               // expire in 10 blocks,
  "Large Cheese Pizza".to_string()  // and get the order right.
)?;

// We can now subscribe to updates to the pizza invoice.
let subscriber = payment_gateway.subscribe(invoice_id)?
  .expect("invoice doesn't exist");

// Have we been paid yet?
let update = subscriber.recv().await.expect("channel closed");

if update.is_confirmed() {
  // Great, ship the pizza and stop tracking the invoice.
  println!("Invoice for \"{}\" paid", update.description());
  payment_gateway.remove_invoice(invoice_id)?;
}   
```
For more detailed documentation, see [docs.rs](https://docs.rs/acceptxmr) or the
[examples](./examples/).