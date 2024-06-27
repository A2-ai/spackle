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
  -D, --dir <DIR>  The directory of the spackle project. Defaults to the current directory [default: .]
  -o, --out <OUT>  The directory to render to. Defaults to 'render' within the current directory. Cannot be the same as the project directory [default: render]
  -v, --verbose    Whether to run in verbose mode
  -h, --help       Print help
  -V, --version    Print version
```

## Project configuration

A spackle project is defined by a `spackle.toml` file at the root directory. Slots defined in the configuration will
fill files templated with a `.j2` extension.

Visit the below page for a full manual on how to configure a spackle project:

### [Configuration manual](docs/configuration.md)

## Contributing

`cargo run`
