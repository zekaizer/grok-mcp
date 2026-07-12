# grok-mcp developer tasks. `make check` is the pre-commit gate.

.PHONY: check fmt lint test build clean

check: fmt lint test ## Full gate: formatting, clippy, tests

fmt: ## Verify formatting
	cargo fmt --all --check

lint: ## Clippy all targets
	cargo clippy --workspace --all-targets -- -D warnings

test: ## Unit / lib tests
	cargo test --workspace --quiet

build: ## Release binary
	cargo build --release -p grok-server --quiet

clean: ## Remove build artifacts
	cargo clean
