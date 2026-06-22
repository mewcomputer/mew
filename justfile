# mew — justfile

set dotenv-load := true
set positional-arguments := true

# Default recipe: build the binary
build:
    cargo build --release -p mew

# Run all tests
test:
    cargo test --all

# Run tests with verbose output
test-v:
    cargo test --all -- --nocapture

mew *args: build
    cargo run -p mew -- "$@"

# Build and run mew. All args after "run" are forwarded to the binary.
# Usage: just run --model deepseek-v4-flash "hello world"
run *args: build
    cargo run -p mew -- run "$@"

# Install to ~/.cargo/bin
install:
    cargo install --path crates/mew

# Install to /usr/local/bin (requires sudo)
install-system: build
    sudo cp target/release/mew /usr/local/bin/mew

# Clean build artifacts
clean:
    cargo clean

# Format all Rust code
fmt:
    cargo fmt

# Run clippy
clippy:
    cargo clippy --all -- -D warnings

# CI-ready check: format, clippy, test
ci: fmt clippy test

# Record a new provider fixture (set MEW_RECORD=1 and provider creds)
record:
    MEW_RECORD=1 cargo test -p mew-provider-openai

# Show module dependencies
deps:
    cargo tree

# Update dependencies
tidy:
    cargo update

site-dev:
    cd site && pnpm install && pnpm run dev
