#!/usr/bin/env bash
# Build both contracts (attestation + escrow) to wasm binaries.
set -euo pipefail

cd "$(dirname "$0")/.."

cargo build --target wasm32v1-none --release
echo "Built: target/wasm32v1-none/release/attestation_contract.wasm"
echo "Built: target/wasm32v1-none/release/attestation_escrow_contract.wasm"