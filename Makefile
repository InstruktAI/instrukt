.PHONY: hooks commit-checks format lint ratchet test audit coverage check dev

# Point git at the repository's checked-in hooks. pre-commit runs the checks the
# staged files need (.pre-commit-config.yaml); the functional suite runs in CI.
hooks:
	scripts/enable-hooks.sh

# Every commit check over every file, as CI runs it.
commit-checks:
	scripts/commit-checks.sh --all-files

format:
	cargo fmt --all

lint:
	scripts/lint.sh

# Lower the suppression baseline to the tree; it never raises an entry.
ratchet:
	scripts/check-ratchet.py --update

test:
	scripts/check-test-tool.sh nextest
	cargo nextest run --workspace --locked
	cargo test --workspace --locked --doc

audit:
	scripts/check-test-tool.sh deny
	cargo deny --locked check

coverage:
	scripts/check-contracts.sh

check: lint test coverage audit

# Run the current development checkout with a private instrukt home.
dev:
	scripts/dev-instrukt $(ARGS)
