#!/usr/bin/env bash

set -euo pipefail

for argument in "$@"; do
    if [[ "$argument" == "--release" ]]; then
        release_build=true
        break
    fi
done

if [[ "${release_build:-false}" != "true" ]]; then
    exec cargo "$@"
fi

site_root="${LEPTOS_SITE_ROOT:-dist}"
pkg_dir="${LEPTOS_SITE_PKG_DIR:-pkg}"
output_name="${LEPTOS_OUTPUT_NAME:-patchwork_web}"
bundle_dir="$site_root/$pkg_dir"

# cargo-leptos starts its frontend and server jobs concurrently. Wait until the
# generated bundle has settled so release builds embed the complete dist tree.
deadline=$((SECONDS + 600))
previous_snapshot=""
stable_snapshots=0

while (( SECONDS < deadline )); do
    if [[ -s "$bundle_dir/$output_name.js" \
        && -s "$bundle_dir/$output_name.wasm" \
        && -s "$bundle_dir/$output_name.css" ]]; then
        snapshot="$({
            find "$site_root" -type f -printf '%P\t%s\t%T@\n'
        } | sort | sha256sum)"

        if [[ "$snapshot" == "$previous_snapshot" ]]; then
            ((stable_snapshots += 1))
        else
            previous_snapshot="$snapshot"
            stable_snapshots=0
        fi

        if (( stable_snapshots >= 4 )); then
            exec cargo "$@"
        fi
    fi

    sleep 0.1
done

printf 'Timed out waiting for cargo-leptos frontend assets in %s\n' "$site_root" >&2
exit 1
