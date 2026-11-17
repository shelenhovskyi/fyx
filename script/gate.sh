#!/usr/bin/env bash
# Merge gate for the integration branch: nothing red gets merged.
# Usage: script/gate.sh [base-ref]   (default base: the upstream tag in FYX_UPSTREAM)
#        FULL=1 script/gate.sh       also builds the `zed` binary (W0 only, on a schedule)
# Gates only crates under crates/ and extensions/ that changed since base; never crates/collab
# (server-only, not built by Fyx) and never docs/.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
base="${1:-$(cat FYX_UPSTREAM)}"

# Workspace-wide checks (spec decision 27). They do not depend on which crates changed, so they
# run first and on every gate, even one with no crate changes. See docs/fyx/ci.md.
if ! script/fyx-check-wasmtime-policy; then
    echo "gate: RED: a non-Cranelift wasmtime backend is enabled (see the lines above)"
    exit 1
fi
if command -v cargo-deny >/dev/null 2>&1; then
    if ! cargo deny --locked --config script/fyx-deny.toml check --hide-inclusion-graph advisories bans 2>&1; then
        echo "gate: RED: cargo deny found an open advisory or a banned wasmtime backend. Update the"
        echo "gate: crate, or triage an upstream-only advisory in script/fyx-deny.toml (docs/fyx/ci.md)."
        echo "gate: Rerun without --hide-inclusion-graph to see which crates pull it in."
        exit 1
    fi
elif [[ "${CI:-}" == "true" ]]; then
    echo "gate: RED: cargo-deny is not installed; CI must run the advisory check"
    exit 1
else
    echo "gate: WARNING: cargo-deny is not installed, so the advisory check is SKIPPED here (CI runs it)."
    echo "gate: WARNING: install it with: cargo install cargo-deny --locked"
fi
# The generated extension API reference (docs/fyx/api) must match the WIT and the SDK, whatever
# crates changed: a WIT or SDK rename must not slip through a gate that skips ui_ext_sdk.
if ! script/fyx-api-docs --check; then
    echo "gate: RED: docs/fyx/api is out of date; run script/fyx-api-docs and commit the pages"
    exit 1
fi

changed_crates=$(
    git diff --name-only "$base"...HEAD -- crates extensions ':!crates/collab' \
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
# Known local issue (docs/fyx/ci.md, "Known local issues"): on a machine whose DNS answers
# example.com with a loopback address, upstream's fetch-tool redirect tests fail because they
# resolve example.com for real. Skip exactly those, loudly, and never in CI.
nextest_filter=()
if [[ "${CI:-}" != "true" && " ${pkgs[*]} " == *" agent "* ]] \
    && powershell -NoProfile -Command "exit [int](-not ((Resolve-DnsName example.com -Type A -ErrorAction SilentlyContinue).IPAddress -match '^127\.'))" 2>/dev/null; then
    echo "gate: WARNING: example.com resolves to loopback here; skipping agent's 3 fetch-tool redirect tests (CI runs them)"
    nextest_filter=(-E 'not (test(test_fetch_tool_follows_same_host_redirect) | test(test_fetch_tool_refuses_redirect_to_loopback) | test(test_fetch_tool_reauthorizes_redirect_to_new_host))')
fi
cargo nextest run "${pkgs[@]}" --no-tests=pass "${nextest_filter[@]}" 2>&1 | tail -30
if [[ "$(git config --bool core.symlinks || echo true)" == "false" ]]; then
    script/fyx-check-licenses-index # symlinks checked out as text files (Windows without Developer Mode)
else
    script/check-licenses
fi

if [[ "${FULL:-0}" == "1" ]]; then
    cargo build -p zed 2>&1 | grep -E '^(error|warning: unused)|Finished' | tail -20
fi

# A lockfile that cargo rewrote during the run is not committed, so `--locked` builds of a fresh
# checkout fail (judge integration-2, finding 1). Includes fixture guests' lockfiles.
if ! git diff --quiet HEAD -- Cargo.lock '**/Cargo.lock'; then
    echo "gate: RED: lockfiles differ from HEAD; commit them:"
    git diff --stat HEAD -- Cargo.lock '**/Cargo.lock'
    exit 1
fi
echo "gate: GREEN"
