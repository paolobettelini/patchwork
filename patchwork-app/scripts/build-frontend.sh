#!/usr/bin/env sh
set -eu

script_dir="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
app_dir="$(dirname "$script_dir")"

cd "$app_dir"
cargo leptos build --release --frontend-only
cp index.html dist/index.html
