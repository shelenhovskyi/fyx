#!/usr/bin/env bash
# SessionStart hook: writes environment for every later Bash tool call into $CLAUDE_ENV_FILE.
# - PATH additions that exist on this machine (~/.cargo/bin, CMake, WinGet Links; rustup on macOS);
# - CARGO_TARGET_DIR from .claude/fyx-targets.json, keyed by the worktree dir name, else its "*"
#   entry (missing dir -> not set);
# - CARGO_BUILD_JOBS (all cores: one build at a time) on Windows only.
# Never fails a session: any error exits 0 silently. Pure bash, no subprocesses (they cost
# ~30-50 ms each on Windows). PowerShell tool calls ignore this file's PATH/dir values.
trap 'exit 0' ERR
[[ -n "${CLAUDE_ENV_FILE:-}" ]] || exit 0

input=""
IFS= read -r -d '' -t 1 input 2>/dev/null || true
cwd=""
if [[ "$input" =~ \"cwd\"[[:space:]]*:[[:space:]]*\"([^\"]*)\" ]]; then cwd="${BASH_REMATCH[1]}"; fi
[[ -n "$cwd" ]] || cwd="${CLAUDE_PROJECT_DIR:-$PWD}"
cwd="${cwd//\\\\//}"   # JSON-escaped backslashes
cwd="${cwd//\\//}"

windows=""
case "${OSTYPE:-}" in msys* | cygwin* | win*) windows=1 ;; esac

# Windows path (C:\x or C:/x) -> /c/x for Git Bash; anything else unchanged.
to_unix() {
    local p="${1//\\//}"
    if [[ -n "$windows" && "$p" =~ ^([A-Za-z]):(/.*)?$ ]]; then
        local d="${BASH_REMATCH[1]}" up="ABCDEFGHIJKLMNOPQRSTUVWXYZ" lo="abcdefghijklmnopqrstuvwxyz" i
        i="${up%%"$d"*}"                      # index of d in up (bash 3.2: no ${d,,})
        ((${#i} < 26)) && d="${lo:${#i}:1}"
        p="/$d${BASH_REMATCH[2]}"
    fi
    printf '%s' "$p"
}

out=""
add_path() {
    local dir="$1"
    [[ -n "$dir" && -d "$dir" ]] || return 0
    out+="case \":\$PATH:\" in *\":$dir:\"*) ;; *) export PATH=\"\$PATH:$dir\" ;; esac"$'\n'
}

add_path "$(to_unix "${HOME:-}")/.cargo/bin"
if [[ -n "$windows" ]]; then
    add_path "/c/Program Files/CMake/bin"
    [[ -n "${LOCALAPPDATA:-}" ]] && add_path "$(to_unix "$LOCALAPPDATA")/Microsoft/WinGet/Links"
elif [[ "${OSTYPE:-}" == darwin* ]]; then
    add_path "/opt/homebrew/opt/rustup/bin"
fi

# Worktree dir name: the path segment after /.claude/worktrees/.
name=""
if [[ "$cwd" =~ /\.claude/worktrees/([^/]+) ]]; then name="${BASH_REMATCH[1]}"; fi
# A worktree's own entry wins; "*" covers every other folder, the main checkout included.
if [[ -z "${CARGO_TARGET_DIR:-}" ]]; then
    src="${BASH_SOURCE[0]//\\//}"
    map="${src%/*}/../fyx-targets.json"
    if [[ -f "$map" ]]; then
        json="$(<"$map")"
        target=""
        if [[ -n "$name" && "$json" =~ \"$name\"[[:space:]]*:[[:space:]]*\"([^\"]+)\" ]]; then
            target="${BASH_REMATCH[1]}"
        elif [[ "$json" =~ \"\*\"[[:space:]]*:[[:space:]]*\"([^\"]+)\" ]]; then
            target="${BASH_REMATCH[1]}"
        fi
        [[ -n "$target" && -d "$(to_unix "$target")" ]] && out+="export CARGO_TARGET_DIR='$target'"$'\n'
    fi
fi

# One build at a time (shared target dir), so it may use every core.
if [[ -n "$windows" && -z "${CARGO_BUILD_JOBS:-}" ]]; then
    out+="export CARGO_BUILD_JOBS=${NUMBER_OF_PROCESSORS:-12}"$'\n'
fi

[[ -n "$out" ]] && { printf '%s' "$out" >> "$CLAUDE_ENV_FILE"; } 2>/dev/null
exit 0
