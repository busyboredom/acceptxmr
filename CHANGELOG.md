# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as described in [The Cargo Book](https://doc.rust-lang.org/cargo/reference/manifest.html#the-version-field).

## [Unreleased]

### Added
- Add a `status()` method to `PaymentGateway` for determining whether the `PaymentGateway` is
  already running.
- Add a `stop()` method to `PaymentGateway` so that it can be gracefully shut down.
- Add a `uri()` method to `Invoice`s which returns a valid monero URI as a string. The URI
  auto-fills the amount due for the end user.
- Implement `Future` for `Subscriber`.

### Changed
- Replace Reqwest with Hyper to improve compile time.
- Increase MSRV to 1.61.
- Update dependencies.
- Use primary address instead of public spend key when creating `PaymentGateway`s.
- Change some parameter types in API to make expensive memory operations like `Clone`s more
  apparent.
- Reduce demo address loading time by getting the address immediately rather than waiting for the
  first invoice update.
- Return an error when attempting to run a `PaymentGateway` which is already running.

### Fixed
- Fix a bug where a change in the order of transactions in the txpool would cause all relevant
  invoices to update, even though no new relevant transactions had been added.

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
