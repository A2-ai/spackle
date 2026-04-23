# 🚰 spackle

A frictionless project templating tool with support for rich interfacing via the web, CLI, and more.

## Usage

```shell
❯ spackle --help
🚰 spackle

Usage: spackle [OPTIONS] <COMMAND>

Commands:
  info   Gets info on a spackle project including the required inputs and their descriptions
  fill   Fills a spackle project using the provided data
  check  Checks the validity of a spackle project
  help   Print this message or the help of the given subcommand(s)

Options:
  -p, --project <PROJECT_PATH>  The spackle project to use (either a directory or a single file). Defaults to the current directory [default: .]
  -v, --verbose                 Whether to run in verbose mode
  -h, --help                    Print help
  -V, --version                 Print version
```

## Project configuration

A spackle project is defined by a `spackle.toml` file at the root directory. Slots defined in the configuration will
fill files templated with a `.j2` extension.

Visit the below page for a full manual on how to configure a spackle project:

### [Configuration manual](docs/configuration.md)

## Install

```shell
brew install a2-ai/tap/spackle
```

## Development

### Prerequisites

- [Rust](https://rustup.rs/)
- [Bun](https://bun.sh/) — required to build and test the TypeScript module
- [wasm-pack](https://rustwasm.github.io/wasm-pack/installer/) — required to build the WebAssembly targets for the TypeScript module:
  ```shell
  cargo install wasm-pack
  ```

> **Note on wasm-pack:** The rustwasm working group [sunset wasm-pack in July 2025](https://blog.rust-lang.org/inside-rust/2025/07/21/sunsetting-the-rustwasm-github-org/). We continue to use it today because it handles target installation, `wasm-bindgen-cli` version pinning, and `wasm-opt` optimization in a single step. We plan to migrate to a [manual `cargo build` + `wasm-bindgen` + `wasm-opt` pipeline](https://nickb.dev/blog/life-after-wasm-pack-an-opinionated-deconstruction/) in a future release to eliminate the dependency on an archived tool.

### Setup

Install git hooks before your first contribution:

```shell
just setup
```

### Build

#### Native (Rust)

```shell
# Run the CLI
just run -- --help

# Run all Rust tests (spackle / spackle-cli / spackle-wasm)
just test

# Smoke-compile the wasm crate on wasm32-unknown-unknown without wasm-pack
just check-wasm-target

# Install the CLI binary locally
just install
```

#### TypeScript module (`@a2-ai/spackle`)

The TypeScript module lives in `ts/` and wraps the wasm binary. Full consumer docs are in [`ts/README.md`](ts/README.md) and [`docs/ts/`](docs/ts/).

> `wasm-pack` must be installed (see [Prerequisites](#prerequisites)) to run any of the commands below.

```shell
# Build all three wasm-pack targets (nodejs, web, bundler) → ts/pkg/
just build-wasm

# Build wasm targets + emit TypeScript declarations to ts/dist/
just build-wasm-ts

# Run the bun test suite (builds wasm first)
just test-wasm-pkg

# Run the demo script (builds wasm first)
just wasm-demo
```
