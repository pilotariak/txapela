# txapela

[![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://github.com/pilotariak/txapela/blob/main/LICENSE)
[![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/pilotariak/txapela/badge)](https://scorecard.dev/viewer/?uri=github.com/pilotariak/txapela)

A Pilotariak service/application.

## Features

- Rust binary built with the 2024 edition
- Formatting, linting and security checks via `prek`
- CI with `cargo fmt`, `cargo clippy`, `cargo test` and `cargo audit`
- Multi-stage, non-root container image published to GHCR on `main`

## Quick start

Requires a stable Rust toolchain.

```sh
cargo build
cargo test
cargo run
```

Run `make help` for the full list of targets.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.
See [CONTRIBUTING](CONTRIBUTING.md).

## License

See [LICENSE](LICENSE).
