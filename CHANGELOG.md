# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic
Versioning](https://semver.org/spec/v2.0.0.html) as described in [The Cargo
Book](https://doc.rust-lang.org/cargo/reference/manifest.html#the-version-field).

## [Unreleased]

## [0.14.0] - 2024-07-04

### Added
- A batteries-included payment gateway built around the core library.
- `get_ids()` method to invoice stores.
- `is_empty()` method to invoice stores.
- `is_paid()` method to `PaymentGateway`.
- `get_invoice_ids()` method to `PaymentGateway`.
- `build_with_mock_daemon()` method to `PaymentGateway`.

### Changed
- Update MSRV to 1.76
- Replace invoice store `try_iter()` method with `try_for_each()`.

### Fixed
- `is_expired()` returning false when invoice is awaiting configrmation despite it
  being expired.

## [0.13.0] - 2023-07-23

### Added
- `OutputKeyStorage` trait, allowing AcceptXMR to store used output keys for
  burning bug mitigation.
- `HeightStorage` trait, allowing AcceptXMR to store the most recently scanned
  height so that blocks are never skipped after extended downtime.
- `Storage` supertrait over all necessary storage traits.
- `initial_height()` method to `PaymentGatewayBuilder`, allowing users to scan
  from wallet restore height to aid in burning bug mitigation.

### Changed
- Update `indexmap` to version `2.0.0`
- Moved `flush()` method from `InvoiceStorage` trait to the `Storage` trait.

### Fixed
- Duplicate output keys not rejected (i.e. burning bug) -- reported by
  [@spirobel](https://www.github.com/spirobel) and
  [@boog900](https://github.com/Boog900)

## [0.12.1] - 2023-06-23

### Changed
- Use webpki CA roots instead of native for better portability.

### Fixed
- `Invoice`'s `expiration_in()` function returning expiration height instead of
  block difference when called before first scan.
- Documentation not getting built with all features on docs.rs --
  [@hinto-janai](https://www.github.com/hinto-janai)
- Only amount of first owned output was considered -- reported by
  [@spirobel](https://www.github.com/spirobel)

## [0.12.0] - 2023-03-18

### Added
- `InvoiceStorage` trait, allowing library users to use custom storage layers.
- `InMemory` store to be used for testing and other applications where
  persistence is not needed.
- `Sqlite` store as a reliable alternative to `Sled`.
- `account_index()` method to `PaymentGatewayBuilder` for specifying the account
  index to be used. Defaults to `0`.
- `custom_storage` example showcasing new custom storage layer functionality.
- Implement `Ord` and `PartialOrd` for `InvoiceId`.
- `blocking_recv` as a replacement for the previously synchronous `recv`.

### Changed
- Increase MSRV to 1.65.
- Add additional `store` argument to `PaymentGateway::builder` for specifying
  storage layer implementation.
- Move `bincode` (de)serialization behind a feature flag.
- Make `PaymentGateway` generic over `InvoiceStore`.
- Change the return type of the `subscribe()` method of `PaymentGateway` to
  `Option<Subscriber>` instead of `Result<Option<Subscriber>>`.
- Change the `recv()` method of `Subscriber` to return `Option` instead of
  `Result`.
- Make the `recv()` method of `Subscriber` an `async` method. Please use
  `blocking_recv()` if you need to block while waiting.
- Make the `recv_timeout` method of `Subscriber` an `async` method.
- Change the `Output` of `Subscriber`'s `Future` impl to `Option<Invoice>`
  instead of `Result<Invoice>`.

### Removed
- `db_path()` method of `PaymentGateway`. Please choose and construct an
  `InvoiceStore` instead.
- `Iterator` implementation on `Subscriber`.

## [0.11.1] - 2022-09-05

### Changed
- Update monero-rs to 0.18, bringing support for view tags and bulletproofs+.

## [0.11.0] - 2022-08-06

### Added
- Add a `status()` method to `PaymentGateway` for determining whether the
  `PaymentGateway` is already running.
- Add a `stop()` method to `PaymentGateway` so that it can be gracefully shut
  down.
- Add a `uri()` method to `Invoice` which returns a valid monero URI as a
  string. The URI auto-fills the amount due for the end user.
- Add `xmr_requested()` method to `invoice` which returns the amount requested
  in XMR.
- Add `xmr_paid()` method to `invoice` which returns the amount paid in XMR.
- Implement `Future` for `Subscriber`.
- Add TLS support.
- Add daemon login support.

### Changed
- Replace Reqwest with Hyper to improve compile time.
- Increase MSRV to 1.61.
- Update dependencies.
- Use primary address instead of public spend key when creating
  `PaymentGateway`s.
- Change some parameter types in API to make expensive memory operations like
  `Clone`s more apparent.
- Reduce demo address loading time by getting the address immediately rather
  than waiting for the first invoice update.
- Return an error when attempting to run a `PaymentGateway` which is already
  running.

### Fixed
- Fix a bug where a change in the order of transactions in the txpool would
  cause all relevant invoices to update, even though no new relevant
  transactions had been added.

## [0.10.1] - 2021-12-04

### Changed

- Update demo's actix dependencies.

### Fixed

- Crash occurring when payment gateway starts and last height in database is
  very recent.

## [0.10.0] - 2021-11-21

### Added

- Initial release of the library

[Unreleased]: https://github.com/busyboredom/acceptxmr/compare/v0.14.0...HEAD
[0.14.0]: https://github.com/busyboredom/acceptxmr/compare/v0.13.0...v0.14.0
[0.13.0]: https://github.com/busyboredom/acceptxmr/compare/v0.12.1...v0.13.0
[0.12.1]: https://github.com/busyboredom/acceptxmr/compare/v0.12.0...v0.12.1
[0.12.0]: https://github.com/busyboredom/acceptxmr/compare/v0.11.1...v0.12.0
[0.11.1]: https://github.com/busyboredom/acceptxmr/compare/v0.11.0...v0.11.1
[0.11.0]: https://github.com/busyboredom/acceptxmr/compare/v0.10.1...v0.11.0
[0.10.1]: https://github.com/busyboredom/acceptxmr/compare/v0.10.0...v0.10.1
[0.10.0]: https://github.com/busyboredom/acceptxmr/releases/tag/v0.10.0
