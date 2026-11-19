#!/usr/bin/env bash
# PreToolUse hook (Bash, PowerShell): denies a foreground `sleep` / `Start-Sleep` longer than 30 s.
# Polling with long sleeps holds a session for minutes and re-reads its whole context on every
# round. Background commands (run_in_background: true) are allowed.
# Never fails a tool call: any error exits 0 silently (= no decision). Pure bash, no subprocesses.
trap 'exit 0' ERR
shopt -s nocasematch
input=""
IFS= read -r -d '' input || true
[[ -n "$input" ]] || exit 0
[[ "$input" =~ \"run_in_background\"[[:space:]]*:[[:space:]]*true ]] && exit 0
# JSON escapes \n and \t would hide a word boundary before `sleep`.
input="${input//\\n/ }"
input="${input//\\t/ }"

limit=30
long=""
# Seconds for "<number><unit>"; units: s (default), m, h, d, ms. Bash 3.2-safe (macOS): no ${x,,}.
check() {
    local num="$1" unit="$2" whole secs
    whole="${num%%.*}"
    [[ -n "$whole" ]] || whole=0
    whole=$((10#$whole))
    case "$unit" in
        ms) secs=$((whole / 1000)) ;;
        m) secs=$((whole * 60)) ;;
        h) secs=$((whole * 3600)) ;;
        d) secs=$((whole * 86400)) ;;
        *) secs=$whole ;;
    esac
    # A fraction above the limit (30.5) counts as longer.
    if ((secs > limit)) || { ((secs == limit)) && [[ "$num" == *.* && "${num#*.}" =~ [1-9] ]]; }; then
        long=1
    fi
}

rest="$input"
# POSIX sleep (also PowerShell's `sleep` alias, seconds): `sleep 60`, `sleep 1.5m`, `sleep infinity`.
while [[ "$rest" =~ (^|[^A-Za-z0-9_.-])sleep[[:space:]]+([0-9]+(\.[0-9]+)?|infinity)(ms|[smhd])? ]]; do
    if [[ "${BASH_REMATCH[2]}" == infinity ]]; then long=1; else check "${BASH_REMATCH[2]}" "${BASH_REMATCH[4]}"; fi
    rest="${rest#*"${BASH_REMATCH[0]}"}"
done
rest="$input"
# PowerShell: `Start-Sleep 60`, `Start-Sleep -Seconds 60`, `-s 60`, `-Milliseconds 60000`, `-m 60000`.
while [[ "$rest" =~ Start-Sleep[[:space:]]+(-(Seconds|s|Milliseconds|m|Duration)[[:space:]:]+)?([0-9]+(\.[0-9]+)?) ]]; do
    case "${BASH_REMATCH[2]}" in  # nocasematch is on
        milliseconds | m) check "${BASH_REMATCH[3]}" ms ;;
        *) check "${BASH_REMATCH[3]}" s ;;
    esac
    rest="${rest#*"${BASH_REMATCH[0]}"}"
done

if [[ -n "$long" ]]; then
    printf '%s\n' '{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"sleep longer than 30 s is blocked in Fyx sessions: use run_in_background + notification, or Monitor"}}'
fi
exit 0
