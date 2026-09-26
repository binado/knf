format-check:
    cargo fmt --all -- --check

format:
    cargo fmt --all

lint:
    cargo clippy --workspace --all-targets -- -D warnings

test:
    cargo test --workspace

test-py:
    cd crates/knf-py && maturin develop && pytest tests

review:
    cargo insta review

build:
    cargo build --workspace

check:
    cargo check --workspace --all-targets

# --lib: knf-core's lib and knf-cli's bin are both named `knf`.
doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --lib

# Mirrors the CI `test` job, minus the `deps` and `package` jobs.
ci: format-check lint test build doc

# Quick static checks without tests or builds.
check-all: format-check lint check

