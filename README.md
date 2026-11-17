# fast-static-site

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![CI](https://github.com/Trysac/u937-fast-static-site/actions/workflows/ci.yml/badge.svg)](https://github.com/Trysac/u937-fast-static-site/actions/workflows/ci.yml)

Incremental static site generator with a hand-written CommonMark subset, strict templates and a dependency graph that rebuilds only the pages a change touches

## Problem

Describe in two or three sentences the concrete problem this utility solves, who hits it, and what
they do today without it. Replace this paragraph when the first real feature lands.

## Install

```sh
cargo add trysac-fast-static-site --git https://github.com/Trysac/u937-fast-static-site
```

Requires Rust 1.88 or newer. No dependencies outside the standard library.

## Usage

```rust
use fast_static_site::{Error, normalize};

// normalize trims surrounding whitespace and lower-cases the text.
assert_eq!(normalize("  Hello World ").as_deref(), Ok("hello world"));

// Blank input is rejected explicitly instead of returning an empty string.
assert_eq!(fast_static_site::normalize("   "), Err(Error::Blank));
```

## Documentation

- [Architecture](docs/ARCHITECTURE.md) and [decisions](docs/decisions/)
- [Testing](docs/TESTING.md) and [security](docs/SECURITY.md)
- [Project record](docs/PROJECT.md) and [stack](docs/STACK.md)
- [Changelog](CHANGELOG.md) and [contributing guide](CONTRIBUTING.md)

## License

[MIT](LICENSE) (c) 2026 Miguel Gomez
