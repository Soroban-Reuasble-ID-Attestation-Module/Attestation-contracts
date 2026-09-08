#!/usr/bin/env bash
# Build the attestation contract to a wasm binary.
set -euo pipefail

cd "$(dirname "$0")/.."

cargo build --target wasm32v1-none --release
echo "Built: target/wasm32v1-none/release/attestation_contract.wasm"