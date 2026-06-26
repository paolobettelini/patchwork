#!/usr/bin/env sh
set -eu

script_dir="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
app_dir="$(dirname "$script_dir")"

cd "$app_dir"
cargo leptos build --frontend-only
cp index.html dist/index.html

if curl -fsS http://127.0.0.1:1420 >/dev/null 2>&1; then
    echo "Patchwork frontend is already available at http://127.0.0.1:1420"
    exit 0
fi

python3 -m http.server 1420 --bind 127.0.0.1 --directory dist
