[![BuildStatus](https://github.com/busyboredom/acceptxmr/workflows/CI/badge.svg)](https://img.shields.io/github/actions/workflow/status/busyboredom/acceptxmr/ci.yml?branch=main)

# `AcceptXMR`: Accept Monero in Your Application
`AcceptXMR` aims to provide a simple, reliable, and efficient means to track
monero payments.

To track payments, `AcceptXMR` generates subaddresses using your private view
key and primary address. It then watches for monero sent to that subaddress
using a monero daemon of your choosing, updating the UI in realtime and
optionally performing a configurable callback once payment is confirmed.

For a batteries-included payment gateway, see
[`AcceptXMR-Server`](./server/).

For a slim & performant library to use in your rust applications, see
[`AcceptXMR`](./library/).

## Key Advantages
* View pair only, no hot wallet.
* Subaddress based. 
* Pending invoices can be stored persistently, enabling recovery from power
  loss. 
* Number of confirmations is configurable per-invoice.
* Ignores transactions with timelocks.
* Tracks used stealth addresses to mitigate the [burning
  bug](https://www.getmonero.org/2018/09/25/a-post-mortum-of-the-burning-bug.html).
* Payment can occur over multiple transactions.

## Security
`AcceptXMR` is non-custodial, and does not require a hot wallet. However, it
does require your private view key and primary address for scanning outputs. If
keeping these private is important to you, please take appropriate precautions
to secure the platform you run your application on.

Care is taken to protect users from malicious transactions containing timelocks
or duplicate output keys (i.e. the burning bug). For the best protection against
the burning bug, it is recommended that users use a dedicated wallet or account
index for `AcceptXMR` that is not used for any other purpose. The payment
gateway's initial height should also be set to the wallet's restore height.
These measures allow `AcceptXMR` to keep a full inventory of used output keys so
that duplicates can be reliably identified.

Also note that anonymity networks like TOR are not currently supported for RPC
calls. This means that your network traffic will reveal that you are interacting
with the monero network.

## Reliability
`AcceptXMR` strives for reliability, but that attempt may not be successful. It
is young and unproven, and relies on several crates which are undergoing rapid
changes themselves. For example, one of the built-in storage layer
implementations ([`Sled`](https://docs.rs/sled)) is still in beta.

That said, `AcceptXMR` can survive unexpected power loss thanks to the ability
to flush pending invoices to disk each time new blocks/transactions are scanned.
A best effort is made to keep the scanning thread free any of potential panics,
and RPC calls in the scanning thread are logged on failure and repeated next
scan. In the event that an error does occur, the liberal use of logging within
`AcceptXMR` will hopefully facilitate a speedy diagnosis and correction.

Use `AcceptXMR` at your own risk.

## Performance
It is recommended that you host your own monero daemon on the same local
network. Network and daemon slowness are the primary cause of high invoice
update latency in the majority of use cases.

To reduce the average latency before receiving invoice updates, you may also
consider lowering the gateway's scanning interval below the default of 1 second.
If using the `AcceptXMR` library, this can be done using the `scan_interval`
method of the `PaymentGatewayBuilder`. If using the standalone
`AcceptXMR-Server`, the scanning interval can be set in config.

Note that lowering the gateway's scanning interval will do nothing if latency to
your chosen node is slower than the scan interval.

## License
Licensed under either of

 * Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
   http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or
   http://opensource.org/licenses/MIT)

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
