# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as described in [The Cargo Book](https://doc.rust-lang.org/cargo/reference/manifest.html#the-version-field).

## [Unreleased]

### Added
- Add a `status()` method to `PaymentGateway`s for determining whether a `PaymentGateway` is already
  running.
- Add a `stop()` method to `PaymentGateway`s so that they can be gracefully shut down.
- Add a `payment_request()` method to `Invoice`s which returns a valid monero payment request string that
  pre-fills the amount due for the user.

### Changed
- Replace Reqwest with Hyper to improve compile time.
- Increase MSRV to 1.58.
- Update dependencies.
- Use primary address instead of public spend key when creating `PaymentGateway`s.
- Change some parameter types in API to make expensive memory operations like `Clone`s more
  apparent.
- Reduce demo address loading time by getting the address immediately rather than waiting for the
  first invoice update.
- Return an error when attempting to run a `PaymentGateway` which is already running.

### Fixed
- Fix a bug where a change in the order of transactions in the txpool would cause all invoices to
  update, even though no new relevant transactions had been added.

## [0.10.1] - 2021-12-04

### Changed

- Update demo's actix dependencies.

### Fixed

- Crash occurring when payment gateway starts and last height in database is very recent.

## [0.10.0] - 2021-11-21

### Added

- Initial release of the library

[Unreleased]: https://github.com/busyboredom/acceptxmr/compare/v0.10.1...HEAD
[0.10.1]: https://github.com/busyboredom/acceptxmr/compare/v0.10.0...v0.10.1
[0.10.0]: https://github.com/busyboredom/acceptxmr/releases/tag/v0.10.0
