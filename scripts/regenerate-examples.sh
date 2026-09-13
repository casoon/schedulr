#!/usr/bin/env bash
# Runs every example program and captures its output in examples/output/<name>.txt.
# The website (site/) shows these files next to the example source, so rerun this
# script and commit the result whenever an example or the solver behaviour changes.
set -euo pipefail

cd "$(dirname "$0")/.."
mkdir -p examples/output
for source in examples/*.rs; do
  name="$(basename "$source" .rs)"
  cargo run --quiet --example "$name" > "examples/output/$name.txt"
  echo "examples/output/$name.txt"
done
