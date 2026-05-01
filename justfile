# mew — justfile

set dotenv-load := true
set positional-arguments := true

# Default recipe: build the binary
build:
    go build -o bin/mew ./cmd/mew

# Run all tests
test:
    go test ./...

# Run tests with verbose output
test-v:
    go test -v ./...

# Build and run mew. All args after "run" are forwarded to the binary.
# Usage: just run --model deepseek-v4-flash "hello world"
run *args: build
    ./bin/mew run "$@"

# Install to $GOPATH/bin (or ~/go/bin)
install:
    go install ./cmd/mew

# Install to /usr/local/bin (requires sudo)
install-system: build
    sudo cp bin/mew /usr/local/bin/mew

# Clean build artifacts
clean:
    rm -rf bin/

# Format all Go code
fmt:
    gofmt -w .

# Run go vet
vet:
    go vet ./...

# CI-ready check: format, vet, test
ci: fmt vet test

# Record a new provider fixture (set MEW_RECORD=1 and provider creds)
record:
    MEW_RECORD=1 go test ./internal/provider/openai/...

# Show module dependencies
deps:
    go mod graph

# Tidy module
tidy:
    go mod tidy
