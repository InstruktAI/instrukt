.PHONY: hooks format lint test check

# Point git at the repository's checked-in hooks (pre-commit: fmt, clippy, test).
hooks:
	git config core.hooksPath .githooks

format:
	cargo fmt --all

lint:
	shellcheck scripts/*.sh .githooks/pre-commit
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets --locked -- -D warnings

test:
	cargo test --workspace --locked

check: lint test
