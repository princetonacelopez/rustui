# Contributing to RustUI

Thank you for considering contributing to RustUI!

## Governance & License

RustUI is dual-licensed under the MIT License ([LICENSE-MIT](LICENSE-MIT)) and the Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE)). Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work shall be dual licensed as above, without any additional terms or conditions.

## Development Workflow

1. Check your environment with `cargo run -p rustui --bin rustui -- doctor`.
2. Ensure formatting passes: `cargo fmt --all -- --check`.
3. Ensure clippy passes with zero warnings: `cargo clippy --workspace --all-targets -- -D warnings`.
4. Ensure all tests pass: `cargo test --workspace`.

## Commit Convention

We follow Conventional Commits (`feat(crate): ...`, `fix(crate): ...`, `docs: ...`, `test(crate): ...`, `chore: ...`).
