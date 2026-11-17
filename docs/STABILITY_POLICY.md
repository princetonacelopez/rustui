# RustUI Stability and Versioning Policy

## Semantic Versioning

RustUI follows [Semantic Versioning 2.0.0](https://semver.org/):

- **`0.1.x` (Current — Alpha / Post-M8)**: Public APIs across the seven crates are feature-complete against the v1 specification, while minor breaking refinements may occur in `0.1.x` / `0.2.x` releases based on real-world application feedback.
- **`0.2.x`**: Derive macros (`#[derive(Observable)]`), dylib hot-reload development mode, and production hardening.
- **`1.0.0`**: Published after at least two non-trivial desktop applications ship on RustUI and the public API remains stable through a full release cycle.

## Minimum Supported Rust Version (MSRV)

RustUI's MSRV policy is **latest stable Rust minus two minor releases** (currently **`1.98`**, using Rust `2024` edition). MSRV bumps are announced in `CHANGELOG.md` and considered minor-version changes once `1.0.0` is reached.

## Licensing

All crates in the RustUI workspace are dual-licensed under **MIT OR Apache-2.0**.
