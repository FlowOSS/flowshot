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

# Run the CLI with args (e.g. `just run capture full`, `just run -- --help`)
run *ARGS:
	cargo run -p flowshot-cli -- {{ARGS}}

# Open the interactive capture overlay (the GUI mode; Esc cancels)
gui:
	cargo run -p flowshot-cli -- capture

# Open the settings window
settings:
	cargo run -p flowshot-cli -- settings

# Pin an image to the screen (e.g. `just pin /tmp/shot.png`)
pin *ARGS:
	cargo run -p flowshot-cli -- pin {{ARGS}}

# Run the foreground daemon
daemon:
	cargo run -p flowshot-cli -- daemon

# Probe the wayland capture
probe:
	cargo run -p flowshot-capture-wayland --example probe