#!/usr/bin/env bash
# The registry's PR, review and publish steps, run locally against the sample entry with
# throwaway TEST keys, ending with Fyx's registry client accepting the result.
#
# Usage: scripts/dry-run.sh [--fyx-ext BIN] [--keep DIR]
#
# - The Fyx checkout this template sits in (tools/fyx-extensions-template) stands in for the
#   entry's source repository at its commit, and builds `fyx-registry` (FYX_REGISTRY overrides).
# - With --fyx-ext, the build runs `fyx-ext check --deny-warnings`, `fyx-ext test` and
#   `fyx-ext package` like pr.yml. Without it, the demo must already be built
#   (`script/fyx-build-demos`), and the package is made with `registry.py package`, the same
#   layout.
# - Test keys are generated into a temporary folder and deleted at the end; with --keep DIR the
#   published files (never the keys) are copied to DIR.
set -euo pipefail

template=$(cd "$(dirname "$0")/.." && pwd)
fyx_repo=$(cd "$template/../.." && pwd)
fyx_ext=""
keep=""
while [ $# -gt 0 ]; do
    case "$1" in
        --fyx-ext) fyx_ext="$2"; shift ;;
        --keep) keep="$2"; shift ;;
        -h|--help) sed -n '2,15p' "$0"; exit 0 ;;
        *) echo "unknown argument $1" >&2; exit 2 ;;
    esac
    shift
done

py=python3
command -v "$py" > /dev/null 2>&1 && "$py" -c 'import sys' 2> /dev/null || py=python
registry_py() { "$py" "$template/scripts/registry.py" "$@"; }
if [ -n "${FYX_REGISTRY:-}" ]; then
    registry() { "$FYX_REGISTRY" "$@"; }
else
    cargo build -q --manifest-path "$fyx_repo/Cargo.toml" -p fyx_registry --bin fyx-registry
    bin="$(cargo metadata -q --format-version 1 --no-deps --manifest-path "$fyx_repo/Cargo.toml" \
        | "$py" -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')/debug/fyx-registry"
    registry() { "$bin" "$@"; }
fi

work=$(mktemp -d)
cleanup() { rm -rf "$work"; }
trap cleanup EXIT
mkdir -p "$work/keys" "$work/out" "$work/review/before" "$work/review/after/extensions"
step() { printf '\n== %s\n' "$*"; }
check() { # check FILE JQ-FILTER MESSAGE
    if [ "$(jq -r "$2" "$1")" != "true" ]; then
        echo "dry run failed: $3" >&2
        jq . "$1" >&2
        exit 1
    fi
}

step "policy script tests"
(cd "$template/scripts" && "$py" -m unittest -q test_registry)

step "throwaway test keys (never real ones)"
keyset_entries=()
for spec in "test-rel-a release" "test-snap-a snapshot" "test-snap-b snapshot blocklist-revocation" \
            "test-bl-a blocklist"; do
    read -r -a parts <<< "$spec"
    id=${parts[0]}
    seed=$("$py" -c 'import base64, os; print(base64.b64encode(os.urandom(32)).decode())')
    printf '{"key_id": "%s", "seed": "%s"}\n' "$id" "$seed" > "$work/keys/$id.json"
    chmod 600 "$work/keys/$id.json"
    roles=()
    for role in "${parts[@]:1}"; do roles+=(--role "$role"); done
    registry public-key --key-file "$work/keys/$id.json" "${roles[@]}" > "$work/keys/$id.public.json"
    keyset_entries+=("$work/keys/$id.public.json")
done
jq -s '{keys: .}' "${keyset_entries[@]}" > "$work/keyset.json"
registry verify keyset "$work/keyset.json" --online-snapshot-key test-snap-a

step "pr.yml: entry"
entry_file="$template/extensions/fyx-demo-counter.toml"
registry_py entry "$entry_file" > "$work/entry.json"
source_dir="$fyx_repo/$(jq -r .source.path "$work/entry.json")"

step "pr.yml: build (the local checkout stands in for source.commit)"
if [ -n "$fyx_ext" ]; then
    (cd "$source_dir" && "$fyx_ext" check --deny-warnings && "$fyx_ext" test \
        && "$fyx_ext" package --out "$work/out")
    package=$(ls "$work/out"/*.fyxext)
else
    component=$("$py" -c 'import sys, tomllib; print(tomllib.load(open(sys.argv[1], "rb"))["fyx_ui"]["component"])' "$source_dir/extension.toml")
    if [ ! -f "$source_dir/$component" ]; then
        echo "no $source_dir/$component: run script/fyx-build-demos first, or pass --fyx-ext" >&2
        exit 1
    fi
    echo "fyx-ext check and test skipped (no --fyx-ext)"
    package="$work/out/build.fyxext"
    registry_py package "$source_dir" --out "$package"
fi
registry_py release --entry "$work/entry.json" --manifest "$source_dir/extension.toml" \
    --package "$package" > "$work/release.json"
jq '{id, version, tier, capabilities}' "$work/release.json"

step "review.yml: no reviewer configured fails closed"
registry_py verdict --inputs-sha256 "$(sha256sum "$entry_file" | cut -d' ' -f1)" > "$work/verdict-none.json"
check "$work/verdict-none.json" '.verdict == "concern" and .reasons[0].detail == "no reviewer configured"' \
    "an unconfigured reviewer must give concern"
cp "$entry_file" "$work/review/after/extensions/"
cat > "$work/review/pr-owner.json" <<EOF
{"author": {"login": "shelenhovskyi"}, "files": [{"path": "extensions/fyx-demo-counter.toml"}],
 "labels": [], "reviews": []}
EOF
cat > "$work/review/jobs.json" <<'EOF'
{"jobs": [{"name": "entry (extensions/fyx-demo-counter.toml)", "conclusion": "success"},
          {"name": "build (extensions/fyx-demo-counter.toml)", "conclusion": "success"}]}
EOF
decide() { # decide PR VERDICT OUT
    registry_py decision-input --pr "$1" --jobs "$work/review/jobs.json" \
        --before "$work/review/before" --after "$work/review/after" \
        --manifest "$source_dir/extension.toml" --package "$package" --verdict "$2" > "$3.input.json"
    registry_py decide "$3.input.json" > "$3"
}
# A contributor's PR with no reviewer: needs-owner, not mergeable.
sed 's/^owners = .*/owners = ["alice"]/; s/^id     = .*/id     = "counter-sample"/' "$entry_file" \
    > "$work/review/after/extensions/counter-sample.toml"
sed 's/^id = .*/id = "counter-sample"/' "$source_dir/extension.toml" > "$work/review/counter-sample.toml"
cat > "$work/review/pr-alice.json" <<EOF
{"author": {"login": "alice"}, "files": [{"path": "extensions/counter-sample.toml"}],
 "labels": [], "reviews": []}
EOF
registry_py decision-input --pr "$work/review/pr-alice.json" --jobs "$work/review/jobs.json" \
    --before "$work/review/before" --after "$work/review/after" \
    --manifest "$work/review/counter-sample.toml" --package "$package" \
    --verdict "$work/verdict-none.json" > "$work/alice.input.json"
registry_py decide "$work/alice.input.json" > "$work/decision-alice.json"
check "$work/decision-alice.json" '.status == "pending" and .auto_merge == false and (.add_labels | index("needs-owner")) != null and .agent_review == "failure"' \
    "a T0 PR without a reviewer must wait for the owner"
# The same PR with a passing verdict: a new T0 extension auto-merges on green CI.
jq -n --arg sha "x" '{verdict: "pass", reasons: [], inputs_sha256: $sha}' > "$work/raw-pass.json"
registry_py verdict "$work/raw-pass.json" --inputs-sha256 x > "$work/verdict-pass.json"
registry_py decision-input --pr "$work/review/pr-alice.json" --jobs "$work/review/jobs.json" \
    --before "$work/review/before" --after "$work/review/after" \
    --manifest "$work/review/counter-sample.toml" --package "$package" \
    --verdict "$work/verdict-pass.json" > "$work/alice-pass.input.json"
registry_py decide "$work/alice-pass.input.json" > "$work/decision-alice-pass.json"
check "$work/decision-alice-pass.json" '.status == "success" and .auto_merge == true and .tier == "t0"' \
    "a new T0 extension with a passing review must auto-merge"
# The sample itself: a reserved fyx-* id, so it needs the owner; the owner's own PR has it.
decide "$work/review/pr-owner.json" "$work/verdict-none.json" "$work/decision-sample.json"
check "$work/decision-sample.json" '.status == "success" and .tier == "t0"' \
    "the owner's PR for the sample must pass the policy"
registry_py comment "$work/decision-sample.json"

step "publish.yml: sign, index, snapshot (test keys)"
now=$(date +%s)
registry_py stamp "$work/release.json" --published-at "$now" > "$work/stamped.json"
registry sign release "$work/stamped.json" --package "$package" \
    --key-file "$work/keys/test-rel-a.json" --out "$work/signed.json"
registry verify release "$work/signed.json" --package "$package" --keyset "$work/keyset.json"
registry index add --index "$work/out/index.json" --release "$work/signed.json" \
    --keyset "$work/keyset.json"
registry sign snapshot --index "$work/out/index.json" --sequence 1 \
    --key-file "$work/keys/test-snap-a.json" --out "$work/out/snapshot.json"
registry verify index "$work/out/index.json" --keyset "$work/keyset.json" \
    --snapshot "$work/out/snapshot.json"
cp "$package" "$work/out/$(jq -r '.signed.package.url | split("/") | last' "$work/signed.json")"
rm -f "$work/out/build.fyxext"

step "blocklist.yml: sign the block list (test key)"
registry_py blocklist "$template/blocklist/blocklist.toml" --sequence 1 --now "$now" \
    > "$work/blocklist-unsigned.json"
registry sign blocklist "$work/blocklist-unsigned.json" --key-file "$work/keys/test-bl-a.json" \
    --out "$work/out/blocklist.json"
registry verify blocklist "$work/out/blocklist.json" --keyset "$work/keyset.json"

step "Fyx's registry client over the published files"
registry verify client "$work/out" --keyset "$work/keyset.json" --now "$now" | tee "$work/client-now.txt"
grep -q "not installable now: TooNew" "$work/client-now.txt" \
    || { echo "dry run failed: a fresh release must wait for the 24 h age gate" >&2; exit 1; }
registry verify client "$work/out" --keyset "$work/keyset.json" --now "$((now + 25 * 3600))" \
    | tee "$work/client-later.txt"
grep -q "fyx-demo-counter@0.1.0: installable (tier t0), package verified" "$work/client-later.txt" \
    || { echo "dry run failed: the client must install the sample after the age gate" >&2; exit 1; }

step "refresh.yml: re-sign the snapshot of the unchanged index"
mapfile -t revoke < <(registry_py revocations "$work/out/snapshot.json" --keyset "$work/keyset.json")
registry sign snapshot --index "$work/out/index.json" --sequence 2 "${revoke[@]}" \
    --key-file "$work/keys/test-snap-a.json" --out "$work/out/snapshot.json"
registry_py expires-within "$work/out/snapshot.json" --hours 24 --now "$now"
registry verify client "$work/out" --keyset "$work/keyset.json" --now "$((now + 25 * 3600))" > /dev/null

if [ -n "$keep" ]; then
    mkdir -p "$keep"
    cp "$work/out"/* "$work/keyset.json" "$keep/"
    echo "published files (no keys) copied to $keep"
fi
printf '\ndry run ok\n'
