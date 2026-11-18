#!/usr/bin/env bash
# Runs the W5 end-to-end check in a dev build, without input devices, and prints the evidence.
#
# Usage (Git Bash, from the repository root, after `cargo build -p zed`):
#   crates/agent_review/test-agent/run-e2e.sh [FYX_EXE] [SCRATCH_DIR]
#
# It creates a scratch profile and project under SCRATCH_DIR (default D:/scratch/fyx-w5-e2e), starts the
# app with the review script `e2e-full.script` and the scripted agent `review_stub_agent.js`
# (REVIEW_STUB_SCENARIO=full), waits for it to quit (at most 4 minutes), then prints the agent's log,
# the review log lines and the files. Never point SCRATCH_DIR at a real profile or project: it is
# deleted first.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
exe="${1:-${CARGO_TARGET_DIR:-target}/debug/fyx.exe}"
scratch="${2:-D:/scratch/fyx-w5-e2e}"
data="$scratch/data"
project="$scratch/project"
stub_log="$scratch/stub.log"

rm -rf "$scratch"
mkdir -p "$data/config" "$project/docs" "$project/.fyx"
lines='first line\nsecond line\nthird line\nfourth line\nfifth line\n'
for name in demo.txt demo2.txt demo3.txt demo4.txt demo.orig; do
    printf "$lines" > "$project/$name"
done
printf '# lock v1\n' > "$project/Cargo.lock"
printf '# Guide\n' > "$project/docs/guide.md"
printf 'deny = ["*.lock"]\n' > "$project/.fyx/review.toml"

agent_js="$(cygpath -m "$here/review_stub_agent.js" 2>/dev/null || echo "$here/review_stub_agent.js")"
cat > "$data/config/settings.json" <<EOF
{
  "session": { "trust_all_worktrees": true },
  "agent_review": { "enabled": true, "auto_accept": ["docs/**"] },
  "agent_servers": {
    "Review Stub": {
      "type": "custom",
      "command": "node",
      "args": ["$agent_js"],
      "env": { "REVIEW_STUB_SCENARIO": "full", "REVIEW_STUB_LOG": "$stub_log" }
    }
  }
}
EOF

FYX_AGENT_REVIEW_SCRIPT="$here/e2e-full.script" RUST_LOG=info \
    timeout 240 "$exe" --foreground --user-data-dir "$data" "$project" > "$scratch/stdout.log" 2>&1 \
    || echo "exit status $?"

echo "--- agent"
cat "$stub_log" 2>/dev/null || echo "(no agent log)"
echo "--- review"
grep -h -E "agent_review" "$data"/logs/*.log | cut -c1-400 || true
echo "--- files"
for name in demo.txt demo2.txt demo3.txt; do
    echo "== $name"
    diff "$project/demo.orig" "$project/$name" || true
done
for name in demo4.txt moved.txt Cargo.lock docs/guide.md outside.txt .fyx/review.toml; do
    echo "== $name"
    cat "$project/$name" 2>/dev/null || echo "(absent)"
done
