#!/usr/bin/env bash
# Builds the Check Probe for wasm32-wasip2 and puts extension.toml and fyx_ui.wasm in <out dir>.
# Usage: tools/fyx-check-probe/build.sh [<out dir>]   (default: tools/fyx-check-probe/out)
set -euo pipefail

probe_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
out_dir=${1:-"$probe_dir/out"}
manifest="$probe_dir/Cargo.toml"

cargo build --manifest-path "$manifest" --target wasm32-wasip2 --release
target_dir=$(
    cargo metadata --manifest-path "$manifest" --format-version 1 --no-deps \
        | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p' \
        | sed 's#\\#/#g'
)
if [[ -z "$target_dir" ]]; then
    echo "build.sh: could not read the target directory from cargo metadata" >&2
    exit 1
fi
mkdir -p "$out_dir"
cp "$target_dir/wasm32-wasip2/release/fyx_check_probe.wasm" "$out_dir/fyx_ui.wasm"
cp "$probe_dir/extension.toml" "$out_dir/extension.toml"
echo "$out_dir"
