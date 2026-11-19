#!/usr/bin/env bash
# PostToolUse hook (Edit, Write, MultiEdit): runs rustfmt on the edited .rs file.
# One file, not `cargo fmt`: formatting the workspace takes seconds and touches other streams'
# files. rustfmt reads rustfmt.toml from the file's ancestors; --edition covers files outside it.
# Never fails a tool call: a file that does not parse yet (mid-edit) is left as it is.
trap 'exit 0' ERR
input=""
IFS= read -r -d '' input || true
[[ "$input" =~ \"file_path\"[[:space:]]*:[[:space:]]*\"(([^\"\\]|\\.)*)\" ]] || exit 0
file="${BASH_REMATCH[1]}"
file="${file//\\\\//}"   # JSON-escaped backslashes (Windows paths)
[[ "$file" == *.rs && -f "$file" ]] || exit 0

# Hooks do not get the PATH that session-env.sh writes for Bash calls.
rustfmt=""
if command -v rustfmt >/dev/null 2>&1; then
    rustfmt=rustfmt
else
    for candidate in "${HOME:-}/.cargo/bin/rustfmt" "${HOME:-}/.cargo/bin/rustfmt.exe" \
        "${USERPROFILE:-}/.cargo/bin/rustfmt.exe" /opt/homebrew/opt/rustup/bin/rustfmt; do
        if [[ -x "$candidate" ]]; then rustfmt="$candidate"; break; fi
    done
fi
[[ -n "$rustfmt" ]] || exit 0

# The project dir as cwd makes rustup pick rust-toolchain.toml's rustfmt.
cd "${CLAUDE_PROJECT_DIR:-.}" 2>/dev/null || true
"$rustfmt" --edition 2024 -- "$file" >/dev/null 2>&1 || true
exit 0
