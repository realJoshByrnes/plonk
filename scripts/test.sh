#!/usr/bin/env bash
# Full check: native tests (compile errors and failures are reported, not hidden) + wasm build + Node runs.
cd "$(dirname "$0")/.."
out=$(cargo test --release --workspace --no-fail-fast -- --nocapture 2>&1)
echo "$out" | grep -E "x86 snippets|^error|FAILED|panicked" | head -n 12
fails=$(echo "$out" | grep -c -E "^error|FAILED")
passed=$(echo "$out" | grep -E "^test result: ok" | sed -E 's/.* ([0-9]+) passed.*/\1/' | awk '{s+=$1} END {print s}')
echo "native tests: $passed passed, $fails failure lines"
cargo build -q --release --target wasm32-unknown-unknown -p plonk-host 2>&1 | tail -n 20
node web/run_wasm.mjs
node web/run_wine_wasm.mjs
