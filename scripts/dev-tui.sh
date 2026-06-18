#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
target_dir="${CARGO_TARGET_DIR:-"$repo_root/target"}"
dev_dir="$target_dir/debug"
max_gib="${GMUS_TARGET_MAX_GIB:-4}"

if ! [[ "$max_gib" =~ ^[1-9][0-9]*$ ]]; then
    echo "GMUS_TARGET_MAX_GIB must be a positive integer" >&2
    exit 2
fi

clean_if_oversize() {
    local size_kib
    local max_kib=$((max_gib * 1024 * 1024))

    if [[ -d "$dev_dir" ]]; then
        size_kib="$(du -sk "$dev_dir" | awk '{print $1}')"
    else
        size_kib=0
    fi

    if ((size_kib > max_kib)); then
        echo "GMUS dev artifacts exceed ${max_gib} GiB; cleaning the dev profile." >&2
        cargo clean --manifest-path "$repo_root/Cargo.toml" --profile dev
    fi
}

clean_if_oversize
trap clean_if_oversize EXIT

cargo run --manifest-path "$repo_root/Cargo.toml" -- tui "$@"
