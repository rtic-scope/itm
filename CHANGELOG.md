# Change Log

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](http://keepachangelog.com/)
and this project adheres to [Semantic Versioning](http://semver.org/).

## [Unreleased]

### Added
- `itm`: `serial` convenience module for serial device configuration. Gated behind a `"serial"` feature.

### Changed
- `itm-decode`: configures the given file as a serial device on `--freq`.
- `iter::Timestamps`: associate the timestamp data relation (now termed "quality") with a wrapping enum instead.
  For example, a previous `Timestamp { offset: 1, data_relation: Sync }` is now represented as `Timestamp::Sync(1)`.
  This change greatly encourages the downstream user to handle all timestamp qualities.

## [v0.7.0] - 2022-01-17

This large bump in minor is required to break the API of both `itm` v0.3.1 and `itm-decode` v0.6.1, which this crate replaces.

### Changed

- `Decoder::singles` and `Decoder::timestamps` now consume the parent `Decoder` on iterator constuction.

### Fixed

- `serde` derivation and build with `--features serde`.
- Some outdated `README.md` documentation.
- `itm-decode`: only require `--freq` on `--timestamps`.

## [v0.4.0] - 2021-12-03

### Changed

The library has been completely reimplemented, following the development of [`itm-decode`](https://github.com/rtic-scope/itm-decode) (now archived).
This new implementation offers, in addition to the previous implementation, an `Iterator`-based design, more granular enums, synchronization packet support, and timestamp generation of trace packets.

The (missing) `itm-dump` binary has been replaced by `itm-decode` shipped in this repository.

Related topics: https://github.com/rust-embedded/itm/pull/41, https://github.com/rust-embedded/wg/pull/589.

## [v0.3.1] - 2018-07-04

### Fixed

- The output of `itmdump -V` no longer includes empty parentheses at the end.

## [v0.3.0] - 2018-07-04

### Changed

- Moved error handling from error-chain failure.

### Fixed

- Include the crate version in the output of `itmdump -V`
- sporadic EOF errors in follow mode

## [v0.2.1] - 2018-02-25

### Changed

- Flush stdout on each write
- Reduce time between retries in follow mode

## [v0.2.0] - 2018-01-11

### Added

- (library) A `Decoder` abstraction for parsing ITM packets out of `Read`-able sources.
- (library) A `Packet` type that represent ITM packets.
- (cli) The stimulus port to read ITM data from can be changed using the `-s` flag. If the flag is
  omitted the port 0 is used by default.
- (cli) A follow mode (`-F` flag) to keep reading the input file. Use this mode if your OS doesn't
  support named pipes.
- (cli) Support for reading from stdin when no `-f` flag is passed.

### Changed

- [breaking-change][] (cli) the file to read must now be passed as an argument of the `-f` flag

## [v0.1.1] - 2016-10-20

### Changed

- `itmdump` no longer depends on the `mkfifo` command.
- `itmdump`, which normally uses named pipes, now fallbacks to regular files to
  be work on Windows.
- `itmdump` now is restrictive with about the arguments it receives. Before, a
  second argument would simply be ignored, but, now, that has become a hard
  error.
- `itmdump` version output (`-V`) now includes the git commit hash and date.

## v0.1.0 - 2016-10-03

### Added

- `itmdump` tool that parses instrumentation packets from the stimulus port 0
  and dumps the payload to `stdout`.

[Unreleased]: https://github.com/rtic-scope/itm/compare/v0.7.0...HEAD
[v0.7.0]: https://github.com/rtic-scope/itm/compare/v0.4.0...v0.7.0
[v0.4.0]: https://github.com/rtic-scope/itm/compare/v0.3.0...v0.4.0
[v0.3.0]: https://github.com/rtic-scope/itm/compare/v0.2.1...v0.3.0
[v0.2.1]: https://github.com/rtic-scope/itm/compare/v0.2.0...v0.2.1
[v0.2.0]: https://github.com/rtic-scope/itm/compare/v0.1.1...v0.2.0
[v0.1.1]: https://github.com/rtic-scope/itm/compare/v0.1.0...v0.1.1
