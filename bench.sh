#!/usr/bin/env bash

set -euo pipefail
cd "$(dirname "$0")"
CPU="${CPU:-$(bench/env.sh free | tail -1)}"
[ -n "$CPU" ] || { echo "no free physical core: every core has a pinned user thread" >&2; exit 1; }
ROUNDS="${ROUNDS:-1000}"
WARMUP="${WARMUP:-20}"
BIG_EVERY="${BIG_EVERY:-8}"
ISOLATE="${ISOLATE:-1}"

cargo test --release --quiet
toolchain=$(sed -n 's/^channel = "\(.*\)"/\1/p' bench/rust-toolchain.toml)
rustup toolchain list | grep -q "^$toolchain" || rustup toolchain install "$toolchain" --profile minimal
(cd bench && cargo build --release --quiet --bin latency)
python3 -I bench/report.py --selftest

mkdir -p data
stamp=$(date +%Y%m%d-%H%M%S)
samples="$PWD/data/samples-$stamp.jsonl"
meta="$PWD/data/meta-$stamp.json"
cmd=(taskset -c "$CPU" "$PWD/bench/target/release/latency" --out "$samples" --meta "$meta" --rounds "$ROUNDS" --big-every "$BIG_EVERY")
if [ "$ISOLATE" = 1 ]; then
  sudo bench/env.sh run "$CPU" -- "${cmd[@]}"
else
  "${cmd[@]}"
fi
python3 -I -c 'import json, sys; m = json.load(open(sys.argv[1])); assert m["cpu"] == int(sys.argv[2]), m' "$meta" "$CPU"
python3 -I bench/report.py "$samples" --meta "$meta" --warmup "$WARMUP" --out report.html
