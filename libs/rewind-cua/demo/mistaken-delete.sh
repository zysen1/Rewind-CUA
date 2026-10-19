#!/usr/bin/env bash
# Replay the mistaken-delete demo and refresh the GIF.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo run --locked --quiet -- demo --out demo/out
cp demo/out/storyboard.json demo/storyboard.json
cp demo/out/scratch/rewind-mistaken-delete.jsonl demo/mistaken-delete.rewind.jsonl
cp demo/out/scratch/rewind-mistaken-delete.jsonl.tip demo/mistaken-delete.rewind.jsonl.tip
python3 demo/render_gif.py demo/storyboard.json demo/mistaken-delete.gif

echo
echo "Journal: demo/mistaken-delete.rewind.jsonl"
echo "GIF:     demo/mistaken-delete.gif"
echo "Replay:  cargo run --locked --quiet -- replay demo/mistaken-delete.rewind.jsonl"
