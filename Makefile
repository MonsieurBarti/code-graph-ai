.PHONY: setup fmt check

# Install version-controlled git hooks
setup:
	git config core.hooksPath .githooks
	@echo "Git hooks installed. Pre-push checks (fmt + clippy) are now active."

# Format all code
fmt:
	cargo fmt --all

# Run formatting and clippy checks (mirrors CI and pre-push hook)
check:
	cargo fmt --all -- --check && RUSTFLAGS="-Dwarnings" cargo clippy --all-targets --all-features
