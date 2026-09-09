#!/usr/bin/env bash
# Optimize both contract wasms for deployment (reduces size and fees).
set -euo pipefail

cd "$(dirname "$0")/.."

mkdir -p target/optimized
stellar contract optimize \
  --wasm target/wasm32v1-none/release/attestation_contract.wasm \
  --wasm-out target/optimized/attestation_contract.wasm
stellar contract optimize \
  --wasm target/wasm32v1-none/release/attestation_escrow_contract.wasm \
  --wasm-out target/optimized/attestation_escrow_contract.wasm

echo "Optimized: target/optimized/attestation_contract.wasm"
echo "Optimized: target/optimized/attestation_escrow_contract.wasm"
ls -lh target/optimized/