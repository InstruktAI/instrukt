.PHONY: hooks format lint test audit coverage check dev

# Point git at the repository's checked-in hooks (pre-commit: fmt, clippy, test).
hooks:
	git config core.hooksPath .githooks

format:
	cargo fmt --all

lint:
	shellcheck -x scripts/*.sh scripts/dev-instrukt .githooks/pre-commit
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets --locked -- -D warnings

test:
	scripts/check-test-tool.sh nextest
	cargo nextest run --workspace --locked
	cargo test --workspace --locked --doc

audit:
	scripts/check-test-tool.sh deny
	cargo deny --locked check

coverage:
	scripts/check-coverage.sh
	scripts/check-committed-config.sh

check: lint test coverage audit

# Run the current development checkout with a private instrukt home.
dev:
	scripts/dev-instrukt $(ARGS)
