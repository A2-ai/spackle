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

```shell
# Setup git hooks
just setup

# Run the CLI
just run -- --help

# Run tests
just test

# Install locally
just install
```

## Typescript Module

Spackle has been rewritten to be able to be translated to a Typescript module. Please find the README for that module here: [`ts/README.md`](ts/README.md)
