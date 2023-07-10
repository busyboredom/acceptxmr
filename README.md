[![BuildStatus](https://github.com/busyboredom/acceptxmr/workflows/CI/badge.svg)](https://img.shields.io/github/actions/workflow/status/busyboredom/acceptxmr/ci.yml?branch=main)
[![Crates.io](https://img.shields.io/crates/v/acceptxmr.svg)](https://crates.io/crates/acceptxmr)
[![Documentation](https://docs.rs/acceptxmr/badge.svg)](https://docs.rs/acceptxmr)
[![MSRV](https://img.shields.io/badge/MSRV-1.65.0-blue)](https://blog.rust-lang.org/2022/11/03/Rust-1.65.0.html)

# `AcceptXMR`: Accept Monero in Your Application

This library aims to provide a simple, reliable, and efficient means to track monero payments.

To track payments, the `PaymentGateway` generates subaddresses using your private view key and
primary address. It then watches for monero sent to that subaddress using a monero daemon of your
choosing, your private view key and your primary address.

Use this library at your own risk, it is young and unproven.

## Key Advantages
* View pair only, no hot wallet.
* Subaddress based. 
* Pending invoices can be stored persistently, enabling recovery from power loss. 
* Number of confirmations is configurable per-invoice.
* Ignores transactions with timelocks.
* Tracks used stealth addresses to mitigate the [burning
  bug](https://www.getmonero.org/2018/09/25/a-post-mortum-of-the-burning-bug.html).
* Payment can occur over multiple transactions.

## Security

`AcceptXMR` is non-custodial, and does not require a hot wallet. However, it does require your
private view key and primary address for scanning outputs. If keeping these private is important
to you, please take appropriate precautions to secure the platform you run your
application on.

Care is taken to protect users from malicious transactions containing timelocks
or duplicate output keys (i.e. the burning bug). For the best protection against
the burning bug, it is recommended that users use a dedicated wallet or account
index for AcceptXMR that is not used for any other purpose. The payment
gateway's initial height should also be set to the wallet's restore height.
These measures allow AcceptXMR to keep a full inventory of used output keys so
that duplicates can be reliably identified.

Also note that anonymity networks like TOR are not currently supported for RPC calls. This
means that your network traffic will reveal that you are interacting with the monero network.

## Reliability

This library strives for reliability, but that attempt may not be successful. `AcceptXMR` is young
and unproven, and relies on several crates which are undergoing rapid changes themselves For
example, the primary storage layer implementation ([`Sled`](https://docs.rs/sled)) is still in beta.

That said, this payment gateway should survive unexpected power loss thanks to the ability to flush
pending invoices to disk each time new blocks/transactions are scanned. A best effort is made to
keep the scanning thread free any of potential panics, and RPC calls in the scanning thread are
logged on failure and repeated next scan. In the event that an error does occur, the liberal use of
logging within this library will hopefully facilitate a speedy diagnosis and correction.

Use this library at your own risk.

## Performance

It is strongly recommended that you host your own monero daemon on the same local network. Network
and daemon slowness are the primary cause of high invoice update latency in the majority of use
cases.

To reduce the average latency before receiving invoice updates, you may also consider lowering
the `PaymentGateway`'s `scan_interval` below the default of 1 second:
```rust
use acceptxmr::PaymentGateway;
use std::time::Duration;

let private_view_key = 
  "ad2093a5705b9f33e6f0f0c1bc1f5f639c756cdfc168c8f2ac6127ccbdab3a03";
let primary_address = 
  "4613YiHLM6JMH4zejMB2zJY5TwQCxL8p65ufw8kBP5yxX9itmuGLqp1dS4tkVoTxjyH3aYhYNrtGHbQzJQP5bFus3KHVdmf";

let store = InMemory::new();

let payment_gateway = PaymentGateway::builder(
  private_view_key.to_string(), 
  primary_address.to_string(), 
  store
)
.scan_interval(Duration::from_millis(100)) // Scan for updates every 100 ms.
.build()?;
```

Please note that `scan_interval` is the minimum time between scanning for updates. If your
daemon's response time is already greater than your `scan_interval` or if your CPU is unable to
scan new transactions fast enough, reducing your `scan_interval` will do nothing.

## License

Licensed under either of

 * Apache License, Version 2.0
   ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license
   ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.

### Donations

AcceptXMR is a hobby project which generates no revenue for the developer(s).
Donations from generous users and community members help keep it economically
viable to work on.

XMR:
`82assiV5dy7guoxxV7vSReZTyY5rGMrWg6BsfvFqiEKRcTiDs7LGMpg5dF5gXVGUWPEXQxyt8SNYx8L8HiGAzvtBK3eJ3EY`
