# Attestation contract developer workflow.
#
# Requires:
#   - Rust stable + wasm32-unknown-unknown target (see rust-toolchain.toml)
#   - `stellar` CLI v21+ (https://github.com/stellar/stellar-cli) for
#     optimize/deploy/invoke targets

CONTRACT_NAME := attestation_contract
ESCROW_NAME := attestation_escrow_contract
WASM := target/wasm32v1-none/release/$(CONTRACT_NAME).wasm
ESCROW_WASM := target/wasm32v1-none/release/$(ESCROW_NAME).wasm

.PHONY: build test clippy fmt wasm optimize deploy-testnet deploy-escrow-testnet invoke-testnet clean

build:
	cargo build

test:
	cargo test --workspace

clippy:
	cargo clippy --workspace --all-targets -- -D warnings

fmt:
	cargo fmt --all -- --check

## Compile both contracts to wasm binaries (release profile, size-optimized).
## Uses wasm32v1-none, required by Soroban env for Rust 1.82+ (wasm32-unknown-unknown
## enables reference-types/multi-value that the Soroban VM does not support).
wasm:
	cargo build --target wasm32v1-none --release

## Produce optimized wasm binaries for both contracts in ./target/optimized/.
optimize: wasm
	mkdir -p target/optimized
	stellar contract optimize --wasm $(WASM) --wasm-out target/optimized/$(CONTRACT_NAME).wasm
	stellar contract optimize --wasm $(ESCROW_WASM) --wasm-out target/optimized/$(ESCROW_NAME).wasm

## Deploy the attestation contract to testnet and write deployments/testnet.json.
## Requires: STELLAR_SOURCE_ACCOUNT (funded testnet identity name).
deploy-testnet: optimize
	./scripts/deploy-testnet.sh

## Deploy the attestation-gated escrow to testnet and append it to deployments/testnet.json.
## Requires: STELLAR_SOURCE_ACCOUNT, ESCROW_ASSET, ESCROW_SUBJECT,
##            ESCROW_CLAIM_TYPE, ESCROW_BENEFICIARY (see the script header).
deploy-escrow-testnet: optimize
	./scripts/deploy-escrow-testnet.sh

## Invoke a contract function on testnet, e.g.:
##   make invoke-testnet ARGS="--id <CONTRACT_ID> --fn verify --arg '...'"
invoke-testnet:
	./scripts/invoke-testnet.sh $(ARGS)

clean:
	cargo clean
	rm -rf target/optimized