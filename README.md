# `AcceptXMR`: A Library for Accepting Monero

This library aims to provide a simple, reliable, and efficient means to track monero payments.

To track a payments, the `PaymentGateway` generates subaddresses using your private view key and
public spend key. It then watches for monero sent to that subaddress by periodically querying a
monero daemon of your choosing, and scanning newly received transactions for relevant outputs
using your private view key and public spend key.

## Security

`AcceptXMR` is non-custodial, and does not require a hot wallet. However, it does require your
private view key and public spend key for scanning outputs. If keeping these private is important
to you, please take appropriate precautions to secure the platform you run your application on.

Also note that anonymity networks like TOR are not currently supported for RPC calls. This
means that your network traffic will reveal that you are interacting with the monero network.

## Reliability

This library strives for reliability, but that attempt may not be successful. `AcceptXMR` is
young and unproven, and relies on several crates which are undergoing rapid changes themselves
(for example, the database used ([`Sled`](https://docs.rs/sled)) is still in beta).

That said, this payment gateway should survive unexpected power loss thanks to pending payments
being stored in a database, which is flushed to disk each time new blocks/transactions are
scanned. A best effort is made to keep the scanning thread free any of potential panics, and RPC
calls in the scanning thread are logged on failure and repeated next scan. In the event that an
error does occur, the liberal use of logging within this library will hopefully facilitate a
speedy diagnosis an correction.

## Performance

For maximum performance, please host your own monero daemon the same local network. Network and
daemon slowness are primary cause of high payment update latency in the majority of use cases.

To reduce the average latency before receiving payment updates, you may also consider lowering
the [`PaymentGateway`]'s `scan_interval` below the default of 1 second:
```
use acceptxmr::PaymentGateway;
use std::time::Duration;

let private_viewkey = "ad2093a5705b9f33e6f0f0c1bc1f5f639c756cdfc168c8f2ac6127ccbdab3a03";
let public_spendkey = "7388a06bd5455b793a82b90ae801efb9cc0da7156df8af1d5800e4315cc627b4";

let payment_gateway = PaymentGateway::builder(private_viewkey, public_spendkey)
    .scan_interval(Duration::from_millis(100)) // Scan for payment updates every 100 ms.
    .build();
```

Please note that `scan_interval` is the minimum time before scanning for updates. If your
daemon's response time is already greater than your `scan_interval`, or if your CPU is unable to
scan new transactions fast enough, reducing your `scan_interval` will do nothing.