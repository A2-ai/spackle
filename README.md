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
fill files templated with a `.j2` or `.tera` extension. Template contents are rendered with [Tera](https://keats.github.io/tera/docs/) — see its docs for the full syntax.

Visit the below page for a full manual on how to configure a spackle project:

### [Configuration manual](docs/configuration.md)

## Install

```shell
brew install a2-ai/tap/spackle
```

## Development

### Prerequisites

- [Rust](https://rustup.rs/)
- [Bun](https://bun.sh/) — required to build and test the TS package
- [just](https://github.com/casey/just) — task runner; drives `setup` / `build-wasm` / etc.
- [jq](https://jqlang.org/) — used by `scripts/build-wasm.sh` to pin the wasm-bindgen CLI version from `Cargo.lock`

The wasm toolchain (`wasm32-unknown-unknown` rust target, `wasm-bindgen-cli` pinned to the `Cargo.lock` version, `wasm-opt`) is installed for you by `just setup` on first run.

### Setup

One-shot onboarding — installs git hooks, runs a `cargo check`, installs JS deps, installs the wasm toolchain:

```shell
just setup      # or: just init
```

Re-run `just setup-wasm` alone if you just need to refresh the wasm toolchain without the full bootstrap.

### Build

#### Native (Rust)

```shell
# Run the CLI
just run -- --help

# Run all Rust tests (spackle / spackle-cli / spackle-wasm)
just test

# Install the CLI binary locally
just install
```

#### TS package (`@a2-ai/spackle`)

The TS package lives in `ts/` and consumes the wasm artifact as a `--target web` ESM bundle that runs in modern browsers and Bun. Full consumer docs are in [`ts/README.md`](ts/README.md) and [`docs/ts/`](docs/ts/).

```shell
# Build everything: CLI binary + wasm + TS dist.
just build

# Or build one component at a time:
just build-cli              # target/release/spackle
just build-wasm             # ts/pkg/ (web-target wasm-bindgen + wasm-opt)
just build-ts               # wasm + tsc emit to ts/dist/ (the TS package)

# Run the TS package's bun test suite (builds wasm first)
just test-ts

# Run the demo script (builds wasm first)
just demo-ts
```
