.PHONY: hooks format lint test coverage check dev

# Point git at the repository's checked-in hooks (pre-commit: fmt, clippy, test).
hooks:
	git config core.hooksPath .githooks

format:
	cargo fmt --all

lint:
	shellcheck scripts/*.sh scripts/dev-instrukt .githooks/pre-commit
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets --locked -- -D warnings

test:
	cargo test --workspace --locked

coverage:
	scripts/check-coverage.sh

check: lint test coverage

# Run the current development checkout with a private instrukt home.
dev:
	scripts/dev-instrukt $(ARGS)
