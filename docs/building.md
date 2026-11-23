# Building

Building MemLite requires Rust 1.92 or newer and a native build toolchain.
MemLite uses Unix sockets and file locks for its shared worker.

Supported platforms:

- Linux x86-64: `x86_64-unknown-linux-gnu`
- Linux ARM64: `aarch64-unknown-linux-gnu`
- Apple Silicon, macOS 13.4 or newer: `aarch64-apple-darwin`

On Apple Silicon macOS, set `MACOSX_DEPLOYMENT_TARGET` to `13.4` or newer
before running the Cargo commands below. This matches the minimum version of
the bundled ONNX Runtime objects:

```sh
export MACOSX_DEPLOYMENT_TARGET=13.4
```

## Install with Cargo

```sh
cargo install memlite-mcp
```

Cargo's binary directory must be on your `PATH` to run `memlite`.

## Build from source

Clone the repository and build:

```sh
git clone https://github.com/Aeron/memlite-mcp.git
cd memlite-mcp
cargo build --release --locked
```

Cargo writes the release binary to `target/release/memlite`.
To install it into Cargo's binary directory instead:

```sh
cargo install --path . --locked
```

## Development

With [just](https://just.systems/) installed, run `just` to list available
commands. The recipes default `MACOSX_DEPLOYMENT_TARGET` to `13.4` while
preserving an existing environment override.

```sh
just check
just release
just install
just run --help
just test -- --test-threads=1
just bench --bench toon_format -- --noplot
```

`just check` checks formatting, runs Clippy on all targets, and runs tests.
Build, install, lint, test, and benchmark recipes forward extra arguments to
Cargo. `just run` forwards them to MemLite.
