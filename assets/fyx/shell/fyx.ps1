# Fyx shell integration for PowerShell (plan 5.17), dot-sourced with `-NoExit -Command` after the
# user's profile. Marks prompts and commands with OSC 633 (A prompt, B prompt end, E command line
# with Fyx's nonce, C executed, D finished, P;Cwd). Only a shell Fyx started has the nonce; it is
# removed from the environment here, so programs never see it.

if ($Global:__FyxLoaded -or -not $env:FYX_SHELL_NONCE) { return }
$Global:__FyxLoaded = $true
$Global:__FyxNonce = $env:FYX_SHELL_NONCE
Remove-Item Env:FYX_SHELL_NONCE -ErrorAction SilentlyContinue
$Global:__FyxCommandRan = $false
$Global:__FyxLastHistoryId = (Get-History -Count 1).Id
$Global:__FyxUserPrompt = $function:prompt

# `\` as `\\`; `;` and control characters as `\xAB`.
function Global:__FyxEscape([string]$Text) {
    $out = [System.Text.StringBuilder]::new()
    foreach ($char in $Text.ToCharArray()) {
        if ($char -eq '\') { [void]$out.Append('\\') }
        elseif ($char -eq ';') { [void]$out.Append('\x3b') }
        elseif ([int]$char -lt 0x20 -or [int]$char -eq 0x7f) { [void]$out.Append(('\x{0:x2}' -f [int]$char)) }
        else { [void]$out.Append($char) }
    }
    $out.ToString()
}

function Global:prompt {
    $succeeded = $global:?
    $exitCode = $global:LASTEXITCODE
    $esc = [char]0x1b
    $bel = [char]0x07
    $marks = ''
    $lastId = (Get-History -Count 1).Id
    if ($Global:__FyxCommandRan -or ($null -eq $Global:__FyxReadLine -and $lastId -ne $Global:__FyxLastHistoryId)) {
        $code = if ($succeeded) { 0 } elseif ($exitCode) { $exitCode } else { 1 }
        if ($null -eq $Global:__FyxReadLine) {
            # Without PSReadLine the command line is known only now, from the history.
            $line = (Get-History -Count 1).CommandLine
            $marks += "$esc]633;E;$(__FyxEscape $line);$Global:__FyxNonce$bel$esc]633;C$bel"
        }
        $marks += "$esc]633;D;$code$bel"
    }
    $Global:__FyxCommandRan = $false
    $Global:__FyxLastHistoryId = $lastId
    if ($PWD.Provider.Name -eq 'FileSystem') {
        $marks += "$esc]633;P;Cwd=$(__FyxEscape $PWD.ProviderPath)$bel"
    }
    $userPrompt = if ($Global:__FyxUserPrompt) { & $Global:__FyxUserPrompt } else { "PS $($PWD.Path)> " }
    $global:LASTEXITCODE = $exitCode
    "$marks$esc]633;A$bel$userPrompt$esc]633;B$bel"
}

if (Get-Command PSConsoleHostReadLine -ErrorAction SilentlyContinue) {
    $Global:__FyxReadLine = $function:PSConsoleHostReadLine
    function Global:PSConsoleHostReadLine {
        $line = $Global:__FyxReadLine.Invoke()
        $text = $line -join "`n"
        if ($text.Trim().Length -gt 0) {
            $esc = [char]0x1b
            $bel = [char]0x07
            [Console]::Write("$esc]633;E;$(__FyxEscape $text);$Global:__FyxNonce$bel$esc]633;C$bel")
            $Global:__FyxCommandRan = $true
        }
        $line
    }
}
