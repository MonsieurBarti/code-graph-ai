.PHONY: setup fmt check dev-build dev

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

# Build the devcontainer from scratch and sync Claude config
dev-build:
	devcontainer up --workspace-folder . --build-no-cache --remove-existing-container

# Start the devcontainer and drop into zsh
dev:
	devcontainer up --workspace-folder . && devcontainer exec --workspace-folder . zsh
