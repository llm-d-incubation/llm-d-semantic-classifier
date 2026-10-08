# -------------------------------------------------------------------
# Configuration
# -------------------------------------------------------------------

V ?=

ifneq ($(V),)
  _NOCAPTURE := -- --nocapture
endif

.PHONY: all build release check clean \
	test test-unit \
	verify lint fmt doc audit \
	playground \
	help

# -------------------------------------------------------------------
# All
# -------------------------------------------------------------------

all: build fmt lint test audit

# -------------------------------------------------------------------
# Build
# -------------------------------------------------------------------

build:
	cargo build --workspace --locked

release:
	cargo build --workspace --release

check:
	cargo check --workspace

clean:
	cargo clean

# -------------------------------------------------------------------
# Test
# -------------------------------------------------------------------

test: test-unit

test-unit:
	cargo test --workspace --all-features --locked $(_NOCAPTURE)

# -------------------------------------------------------------------
# Quality
# -------------------------------------------------------------------

# The local gate: fmt, lint, build, tests. Must be GREEN before review.
verify: lint build test

lint:
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets --all-features -- -D warnings

fmt:
	cargo fmt --all

doc:
	RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items

audit:
	cargo deny check

# -------------------------------------------------------------------
# Convenience
# -------------------------------------------------------------------

# Interactive semantic-cache playground (Redis Stack + web UI on :8080).
playground:
	./hack/playground

# -------------------------------------------------------------------
# Help
# -------------------------------------------------------------------

help:
	@echo "Variables:"
	@echo "  V=1   show test output (--nocapture)"
	@echo ""
	@echo "Top-level:"
	@echo "  all       build + fmt + lint + test + audit"
	@echo ""
	@echo "Build:"
	@echo "  build     cargo build --workspace --locked"
	@echo "  release   cargo build --workspace --release"
	@echo "  check     cargo check --workspace"
	@echo "  clean     cargo clean"
	@echo ""
	@echo "Test:"
	@echo "  test      run all tests"
	@echo ""
	@echo "Quality:"
	@echo "  verify    the local gate: fmt + clippy + build + tests"
	@echo "  lint      rustfmt check + clippy -D warnings"
	@echo "  fmt       format with rustfmt"
	@echo "  doc       build docs with warnings denied"
	@echo "  audit     cargo deny check"
	@echo ""
	@echo "Convenience:"
	@echo "  playground  launch the interactive playground"
