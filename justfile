# Build the workspace
build:
	cargo build --workspace

# Run tests on the workspace
test:
	cargo test --workspace

# Run clippy on the workspace
clippy:
	cargo clippy --workspace --all-targets -- -D warnings

# Check formatting
fmt:
	cargo fmt --check

# Run purity gate
purity:
	./scripts/purity-gate.sh

# Run cargo deny check
deny:
	cargo deny check

# Run all checks (fmt + clippy + test + purity + deny)
check:
	cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && ./scripts/purity-gate.sh && cargo deny check

# Run the CLI
run:
	cargo run -p flowshot-cli

# Probe the wayland capture
probe:
	cargo run -p flowshot-capture-wayland --example probe