#!/usr/bin/env bash
# PreToolUse hook (Bash, PowerShell): denies what only the user or the integrator may do.
# - `git push` to a protected branch (also a bare `git push` or `HEAD` while on one), of a tag,
#   with --tags/--follow-tags/--mirror/--all, a force push (-f, --force*, +refspec) and a remote
#   delete (--delete, -d, :refspec). Catches `git -C <dir> push`, which settings.json's deny list
#   cannot.
# - `cargo add/remove/update/generate-lockfile`: they change Cargo.toml/Cargo.lock (W0 only).
#   FYX_INTEGRATOR=1 in the session's env (.claude/settings.local.json "env") allows them.
# Never fails a tool call on its own error: any error exits 0 (= no decision). Runs git only
# when the command has a `push`.
trap 'exit 0' ERR
protected_branches=" main fyx/main "

input=""
IFS= read -r -d '' input || true
[[ "$input" == *push* || "$input" == *cargo* ]] || exit 0

deny() {
    local reason="${1//\"/\'}"
    printf '{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"%s"}}\n' "$reason"
    exit 0
}

[[ "$input" =~ \"command\"[[:space:]]*:[[:space:]]*\"(([^\"\\]|\\.)*)\" ]] || exit 0
command="${BASH_REMATCH[1]}"
cwd=""
if [[ "$input" =~ \"cwd\"[[:space:]]*:[[:space:]]*\"(([^\"\\]|\\.)*)\" ]]; then cwd="${BASH_REMATCH[1]}"; fi
cwd="${cwd//\\\\//}"
[[ -n "$cwd" ]] || cwd="${CLAUDE_PROJECT_DIR:-.}"

command="${command//\\n/$'\n'}"
command="${command//\\t/ }"
command="${command//\\r/ }"
command="${command//\\\"/}"
command="${command//\\\\/\\}"
# Command separators and substitutions start a new segment; quotes are dropped.
for separator in ';' '&' '|' '(' ')' '`' '{' '}'; do command="${command//"$separator"/$'\n'}"; done
command="${command//\'/}"
command="${command//\"/}"

is_protected() { [[ "$protected_branches" == *" ${1#refs/heads/} "* ]]; }

check_push() {
    # $1: git's working dir; the rest: push's arguments.
    local dir="$1" argument positional=() skip=""
    shift
    for argument in "$@"; do
        if [[ -n "$skip" ]]; then skip=""; continue; fi
        case "$argument" in
            --force* | --mirror | --all | --branches | --tags | --follow-tags | --delete | --prune)
                deny "git push $argument is blocked in Fyx sessions: only the user force-pushes, pushes tags or deletes remote branches" ;;
            -o | --push-option | --repo | --receive-pack | --exec) skip=1 ;;
            --*) ;;
            -*)
                if [[ "$argument" =~ ^-[A-Za-z0-9]*[fd] ]]; then
                    deny "git push $argument is blocked in Fyx sessions: only the user force-pushes or deletes remote branches"
                fi
                [[ "$argument" =~ ^-[A-Za-z0-9]*o$ ]] && skip=1
                ;;
            *) positional+=("$argument") ;;
        esac
    done

    local current=""
    current="$(git -C "$dir" symbolic-ref --quiet --short HEAD 2>/dev/null)" || current=""
    if ((${#positional[@]} < 2)); then
        if [[ -n "$current" ]] && is_protected "$current"; then
            deny "git push while on $current is blocked in Fyx sessions: only the user pushes $current"
        fi
        return 0
    fi

    local refspec source destination
    for refspec in "${positional[@]:1}"; do
        if [[ "$refspec" == +* ]]; then
            deny "git push $refspec is a force push, blocked in Fyx sessions"
        fi
        if [[ "$refspec" == :* ]]; then
            deny "git push $refspec deletes a remote ref, blocked in Fyx sessions"
        fi
        if [[ "$refspec" == *'*'* ]]; then
            deny "git push with a wildcard refspec ($refspec) is blocked in Fyx sessions"
        fi
        source="${refspec%%:*}"
        destination="${refspec##*:}"
        [[ "$destination" == HEAD || "$destination" == @ ]] && destination="$current"
        if [[ "$destination" == refs/tags/* || "$source" == refs/tags/* ]]; then
            deny "git push of a tag ($refspec) is blocked in Fyx sessions: only the user pushes tags"
        fi
        if [[ -n "$destination" ]] && is_protected "$destination"; then
            deny "git push to $destination is blocked in Fyx sessions: only the user pushes it"
        fi
        if [[ "$refspec" != *:* && "$source" != HEAD && "$source" != @ ]] \
            && git -C "$dir" show-ref --verify --quiet "refs/tags/$source" 2>/dev/null \
            && ! git -C "$dir" show-ref --verify --quiet "refs/heads/$source" 2>/dev/null; then
            deny "git push of tag $source is blocked in Fyx sessions: only the user pushes tags"
        fi
    done
}

check_segment() {
    local words=() index count dir
    read -r -a words <<<"$1" || true
    count=${#words[@]}
    for ((index = 0; index < count; index++)); do
        case "${words[index]}" in
            git | git.exe | */git | */git.exe | *\\git.exe)
                dir="$cwd"
                local next=$((index + 1))
                while ((next < count)); do
                    case "${words[next]}" in
                        -C)
                            next=$((next + 1))
                            local target="${words[next]:-.}"
                            if [[ "$target" == /* || "$target" =~ ^[A-Za-z]: ]]; then dir="$target"; else dir="$dir/$target"; fi
                            ;;
                        -c | --git-dir | --work-tree | --namespace | --exec-path) next=$((next + 1)) ;;
                        -*) ;;
                        *) break ;;
                    esac
                    next=$((next + 1))
                done
                if [[ "${words[next]:-}" == push ]]; then
                    check_push "$dir" "${words[@]:next+1}"
                fi
                ;;
            cargo | cargo.exe | */cargo | */cargo.exe | *\\cargo.exe)
                [[ "${FYX_INTEGRATOR:-}" == 1 ]] && continue
                local subcommand="" next=$((index + 1))
                while ((next < count)) && [[ "${words[next]}" == [-+]* ]]; do next=$((next + 1)); done
                subcommand="${words[next]:-}"
                case "$subcommand" in
                    add | remove | rm | update | upgrade | generate-lockfile)
                        deny "cargo $subcommand changes Cargo.toml/Cargo.lock, which only the integrator (W0) edits: list the change for the integrator in your progress file" ;;
                esac
                ;;
        esac
    done
}

while IFS= read -r segment; do
    [[ "$segment" == *git* || "$segment" == *cargo* ]] || continue
    check_segment "$segment"
done <<<"$command"
exit 0
