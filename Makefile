SHELL := /bin/sh

COMPREHENSIVE_FEATURES := hyvor-relay,api,rustls,queue-memory,rate-limit,smtp,telemetry,queue-sqlite,queue-postgres,queue-scylla,sendgrid,sendpulse,mailgun,dotenv,test-utils
NATIVE_TLS_FEATURES := hyvor-relay,api,native-tls
CLIPPY_FLAGS := -- -D warnings -W clippy::pedantic
AUDIT_DB := target/advisory-db

.PHONY: dev fmt fmt-check lint clippy test test-ci test-comprehensive doc audit security concurrency perf miri live-postgres live-scylla clean

dev: fmt lint test doc audit

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

lint: clippy

clippy:
	cargo clippy --features queue-sqlite --all-targets $(CLIPPY_FLAGS)
	cargo clippy --no-default-features --features $(COMPREHENSIVE_FEATURES) --all-targets $(CLIPPY_FLAGS)

test:
	cargo test

test-ci: fmt-check
	cargo test
	cargo test --no-default-features
	cargo test --features queue-sqlite
	cargo test --features queue-scylla
	cargo test --features smtp
	cargo test --no-default-features --features $(NATIVE_TLS_FEATURES)
	cargo test --no-default-features --features $(COMPREHENSIVE_FEATURES)

test-comprehensive:
	cargo test --no-default-features --features $(COMPREHENSIVE_FEATURES)

doc:
	cargo doc --no-deps --no-default-features --features $(COMPREHENSIVE_FEATURES)

audit: security concurrency perf

security:
	cargo audit --db $(AUDIT_DB)
	cargo deny --no-default-features --features $(COMPREHENSIVE_FEATURES) check

concurrency:
	cargo test --features queue-memory queue::memory
	cargo test --features queue-sqlite queue::sqlite
	cargo test --features queue-postgres queue::postgres
	cargo test --features queue-scylla queue::scylla

perf:
	cargo test --release --no-default-features --features $(COMPREHENSIVE_FEATURES)

miri:
	cargo +nightly miri test --no-default-features --features queue-memory

live-postgres:
	MAILBRIDGE_RUN_PERSISTENT_TESTS=true cargo test --features queue-postgres queue::postgres

live-scylla:
	MAILBRIDGE_RUN_PERSISTENT_TESTS=true cargo test --features queue-scylla queue::scylla

clean:
	cargo clean
