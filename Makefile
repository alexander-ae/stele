# The checks a pipeline would run. Shared runners on this account are over
# quota, so the same commands live here for a local `make check`.
# rustup was installed without touching the shell profile; put its bin first.
export PATH := $(HOME)/.cargo/bin:$(PATH)

.PHONY: check build

check:
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings
	cargo test

build:
	cargo build --release
