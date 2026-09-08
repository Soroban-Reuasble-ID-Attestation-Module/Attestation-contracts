# Attestation contract developer workflow.
#
# Requires:
#   - Rust stable + wasm32-unknown-unknown target (see rust-toolchain.toml)
#   - `stellar` CLI v21+ (https://github.com/stellar/stellar-cli) for
#     optimize/deploy/invoke targets

CONTRACT_NAME := attestation_contract
WASM := target/wasm32v1-none/release/$(CONTRACT_NAME).wasm

.PHONY: build test clippy fmt wasm optimize deploy-testnet invoke-testnet clean

build:
	cargo build

test:
	cargo test

clippy:
	cargo clippy --all-targets -- -D warnings

fmt:
	cargo fmt --all -- --check

## Compile the contract to a wasm binary (release profile, size-optimized).
## Uses wasm32v1-none, required by Soroban env for Rust 1.82+ (wasm32-unknown-unknown
## enables reference-types/multi-value that the Soroban VM does not support).
wasm:
	cargo build --target wasm32v1-none --release

## Produce an optimized wasm binary in ./target/optimized/.
optimize: wasm
	mkdir -p target/optimized
	stellar contract optimize --wasm $(WASM) --output target/optimized/$(CONTRACT_NAME).wasm

## Deploy the optimized wasm to testnet and write deployments/testnet.json.
## Requires: STELLAR_SOURCE_ACCOUNT (funded testnet account secret key).
deploy-testnet: optimize
	./scripts/deploy-testnet.sh

## Invoke a contract function on testnet, e.g.:
##   make invoke-testnet ARGS="--id <CONTRACT_ID> --fn verify --arg '...'"
invoke-testnet:
	./scripts/invoke-testnet.sh $(ARGS)

clean:
	cargo clean
	rm -rf target/optimized