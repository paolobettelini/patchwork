#!/usr/bin/env sh
set -eu

script_dir="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
app_dir="$(dirname "$script_dir")"

cd "$app_dir"
cargo tauri build --debug --no-bundle --ci
