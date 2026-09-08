#!/usr/bin/env bash
# Optimize the contract wasm for deployment (reduces size and fees).
set -euo pipefail

cd "$(dirname "$0")/.."

mkdir -p target/optimized
stellar contract optimize \
  --wasm target/wasm32-unknown-unknown/release/attestation_contract.wasm \
  --output target/optimized/attestation_contract.wasm

echo "Optimized: target/optimized/attestation_contract.wasm"
ls -lh target/optimized/attestation_contract.wasm