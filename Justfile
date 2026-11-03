set positional-arguments

# Minimum macOS version shared by Rust and native dependencies
export MACOSX_DEPLOYMENT_TARGET := env("MACOSX_DEPLOYMENT_TARGET", "13.4")

# Lists available development commands
default:
    @just --list

# Builds the development binary
build *args:
    cargo build --locked "$@"

# Builds the optimized release binary
release *args:
    cargo build --release --locked "$@"

# Installs the release binary into Cargo's binary directory
install *args:
    cargo install --path . --locked "$@"

# Runs the CLI with the given application arguments
run *args:
    cargo run --locked -- "$@"

# Formats all Rust sources
fmt:
    cargo fmt --all

# Checks Rust formatting without modifying files
fmt-check:
    cargo fmt --all -- --check

# Runs Clippy on all targets with warnings treated as errors
lint *args:
    cargo clippy --all-targets --locked "$@" -- -D warnings

# Runs unit and integration tests with optional Cargo arguments
test *args:
    cargo test --locked "$@"

# Runs Criterion suites with optional Cargo and benchmark arguments
bench *args:
    cargo bench --locked "$@"

# Checks formatting, lints all targets, and runs tests
check: fmt-check lint test

# Removes Cargo build artifacts
clean:
    cargo clean
