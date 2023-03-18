# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic
Versioning](https://semver.org/spec/v2.0.0.html) as described in [The Cargo
Book](https://doc.rust-lang.org/cargo/reference/manifest.html#the-version-field).

## [Unreleased]

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
- Change the `Output` of `Subsciber`'s `Future` impl to `Option<Invoice>`
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

[Unreleased]: https://github.com/busyboredom/acceptxmr/compare/v0.12.0...HEAD
[0.12.0]: https://github.com/busyboredom/acceptxmr/compare/v0.11.1...v0.12.0
[0.11.1]: https://github.com/busyboredom/acceptxmr/compare/v0.11.0...v0.11.1
[0.11.0]: https://github.com/busyboredom/acceptxmr/compare/v0.10.1...v0.11.0
[0.10.1]: https://github.com/busyboredom/acceptxmr/compare/v0.10.0...v0.10.1
[0.10.0]: https://github.com/busyboredom/acceptxmr/releases/tag/v0.10.0
