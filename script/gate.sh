#!/usr/bin/env bash
# Merge gate for the integration branch: nothing red gets merged.
# Usage: script/gate.sh [base-ref]   (default base: the upstream tag in FYX_UPSTREAM)
#        FULL=1 script/gate.sh       also builds the `zed` binary (W0 only, on a schedule)
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
base="${1:-$(cat FYX_UPSTREAM)}"

changed_crates=$(
    git diff --name-only "$base"...HEAD -- crates extensions \
        | awk -F/ '{print $1"/"$2"/Cargo.toml"}' | sort -u \
        | while read -r manifest; do
            [[ -f "$manifest" ]] && sed -n 's/^name = "\(.*\)"/\1/p' "$manifest" | head -1
        done
)

if [[ -z "$changed_crates" ]]; then
    echo "gate: no crate changes since $base"
    exit 0
fi

pkgs=()
for c in $changed_crates; do pkgs+=(-p "$c"); done
echo "gate: ${changed_crates//$'\n'/ }"

cargo check "${pkgs[@]}" 2>&1 | tail -20
cargo clippy "${pkgs[@]}" --all-targets -- --deny warnings 2>&1 | tail -30
cargo nextest run "${pkgs[@]}" --no-tests=pass 2>&1 | tail -30
if [[ "$(git config --bool core.symlinks || echo true)" == "false" ]]; then
    script/fyx-check-licenses-index # symlinks checked out as text files (Windows without Developer Mode)
else
    script/check-licenses
fi

if [[ "${FULL:-0}" == "1" ]]; then
    cargo build -p zed 2>&1 | grep -E '^(error|warning: unused)|Finished' | tail -20
fi
echo "gate: GREEN"
