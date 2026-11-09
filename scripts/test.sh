#!/usr/bin/env bash
# Quiet full check: native snippet tests + wasm build + Node run.
set -e
cd "$(dirname "$0")/.."
cargo test -q --workspace -- --nocapture 2>&1 | grep -E "x86 snippets|FAIL|test result: F|error" | head -n 10 || true
cargo build -q --release --target wasm32-unknown-unknown -p plonk-host 2>&1 | tail -n 20
node web/run_wasm.mjs
node web/run_wine_wasm.mjs
