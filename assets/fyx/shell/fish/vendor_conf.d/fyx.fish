# Fyx shell integration for fish (plan 5.17), found through the XDG_DATA_DIRS entry Fyx adds.
# Marks prompts and commands with OSC 633 (A prompt, B command start, E command line with Fyx's
# nonce, C executed, D finished, P;Cwd). Only a shell Fyx started has the nonce; it is unset
# here, so programs never see it, and Fyx's XDG_DATA_DIRS entry is taken out again.

if set -q FYX_FISH_DATA_DIR
    set -gx XDG_DATA_DIRS (string match -v -- $FYX_FISH_DATA_DIR $XDG_DATA_DIRS)
    set -e FYX_FISH_DATA_DIR
end

if status is-interactive; and set -q FYX_SHELL_NONCE; and not set -q __fyx_loaded
    set -g __fyx_loaded 1
    set -g __fyx_nonce $FYX_SHELL_NONCE
    set -e FYX_SHELL_NONCE
    set -g __fyx_in_command 0

    # `\` as `\\`; `;` and control characters as `\xAB`.
    function __fyx_escape
        set -l out
        for char in (string split -- '' $argv[1])
            if test "$char" = '\\'
                set -a out '\\\\'
            else if test "$char" = ';'
                set -a out '\\x3b'
            else if string match -qr -- '[[:cntrl:]]' $char
                set -a out (printf '\\\\x%02x' "'$char")
            else
                set -a out $char
            end
        end
        string join '' -- $out
    end

    function __fyx_prompt --on-event fish_prompt
        printf '\e]633;P;Cwd=%s\a\e]633;A\a' (__fyx_escape "$PWD")
    end

    function __fyx_preexec --on-event fish_preexec
        set -g __fyx_in_command 1
        printf '\e]633;B\a\e]633;E;%s;%s\a\e]633;C\a' (__fyx_escape "$argv[1]") $__fyx_nonce
    end

    function __fyx_postexec --on-event fish_postexec
        set -l code $status
        if test $__fyx_in_command = 1
            printf '\e]633;D;%s\a' $code
            set -g __fyx_in_command 0
        end
    end
end
