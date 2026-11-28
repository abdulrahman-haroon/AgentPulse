# AgentPulse status-line wrapper for Claude Code. Setup > Connect points `statusLine` here.
# 1. Claude Code pipes its status JSON to us. If it has `rate_limits` (Pro/Max subscribers,
#    after the first reply), the plan-limit numbers are saved to AgentPulse's data folder
#    (claude-limits.json) so the app can show 5-hour / weekly / spend-limit bars.
# 2. The user's own status line (saved by Connect in statusline-original.json next to this
#    script) is run the same way Claude Code would, with the same input, and its output is
#    passed through unchanged. Without one, a short default line is printed.
#
# Only numbers are copied from the input; nothing from it is executed. The command that runs
# is the user's own, saved from their settings.json. Never fails: errors just skip a step.
# Keep this file ASCII (PowerShell 5.1 reads it as ANSI).

$ErrorActionPreference = 'Stop'
$dir = Join-Path $env:LOCALAPPDATA 'dev.agentpulse.app'
$raw = ''
$status = $null

try {
    $stdin = [Console]::OpenStandardInput()
    $raw = (New-Object IO.StreamReader($stdin, [Text.UTF8Encoding]::new($false))).ReadToEnd()
    $status = $raw | ConvertFrom-Json
} catch {}

# 1. Save the plan-limit numbers.
try {
    $rl = $status.rate_limits
    if ($rl) {
        $limits = [ordered]@{}
        foreach ($k in 'five_hour', 'seven_day', 'spend_limit') {
            $w = $rl.$k
            if ($null -eq $w -or $null -eq $w.used_percentage) { continue }
            $o = [ordered]@{ used_percentage = [double]$w.used_percentage }
            if ($null -ne $w.resets_at) { $o.resets_at = [long]$w.resets_at }
            if ($k -eq 'spend_limit') {
                if ($null -ne $w.used_usd) { $o.used_usd = [double]$w.used_usd }
                if ($null -ne $w.limit_usd) { $o.limit_usd = [double]$w.limit_usd }
                if ($w.period -in 'daily', 'weekly', 'monthly') { $o.period = [string]$w.period }
            }
            $limits[$k] = $o
        }
        if ($limits.Count -gt 0) {
            $snap = [ordered]@{ ts = (Get-Date).ToUniversalTime().ToString('o'); rate_limits = $limits }
            [void](New-Item -ItemType Directory -Force $dir)
            $file = Join-Path $dir 'claude-limits.json'
            $tmp = "$file.$PID.tmp"
            [IO.File]::WriteAllText($tmp, ($snap | ConvertTo-Json -Compress -Depth 4), [Text.UTF8Encoding]::new($false))
            Move-Item -Force $tmp $file
        }
    }
} catch {}

# 2. The user's own status line.
$command = $null
try {
    $saved = Join-Path $PSScriptRoot 'statusline-original.json'
    if (Test-Path $saved) {
        $command = [string](Get-Content $saved -Raw | ConvertFrom-Json).command
    }
} catch {}

function Find-GitBash {
    if ($env:CLAUDE_CODE_GIT_BASH_PATH -and (Test-Path $env:CLAUDE_CODE_GIT_BASH_PATH)) { return $env:CLAUDE_CODE_GIT_BASH_PATH }
    $git = Get-Command git.exe -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $git) { return $null }
    $d = Split-Path $git.Source
    for ($i = 0; $i -lt 3 -and $d; $i++) {
        $bash = Join-Path $d 'bin\bash.exe'
        if (Test-Path $bash) { return $bash }
        $d = Split-Path $d
    }
    return $null
}

if ($command) {
    try {
        # Same shell choice as Claude Code on Windows: Git Bash when installed, else PowerShell.
        # The command is never spliced into an argument string: bash reads it from an env var,
        # PowerShell gets it base64-encoded.
        $psi = New-Object Diagnostics.ProcessStartInfo
        $bash = Find-GitBash
        if ($bash) {
            $psi.FileName = $bash
            $psi.Arguments = '-c "eval \"$AGENTPULSE_STATUSLINE_COMMAND\""'
            $psi.EnvironmentVariables['AGENTPULSE_STATUSLINE_COMMAND'] = $command
        } else {
            $psi.FileName = 'powershell.exe'
            # This PowerShell passes our input on to the command. Left to its defaults it re-encodes
            # it (OEM code page, then a BOM) and the user's line gets unreadable JSON.
            $script = '$ProgressPreference = ''SilentlyContinue''; $u = [Text.UTF8Encoding]::new($false); ' +
                '[Console]::InputEncoding = $u; $OutputEncoding = $u; [Console]::In.ReadToEnd() | ' + $command
            $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($script))
            $psi.Arguments = "-NoProfile -NonInteractive -EncodedCommand $encoded"
        }
        $psi.UseShellExecute = $false
        $psi.CreateNoWindow = $true
        $psi.RedirectStandardInput = $true
        $psi.RedirectStandardOutput = $true
        $psi.RedirectStandardError = $true
        $psi.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
        $psi.WorkingDirectory = (Get-Location).ProviderPath
        $p = [Diagnostics.Process]::Start($psi)
        $out = $p.StandardOutput.ReadToEndAsync()
        $null = $p.StandardError.ReadToEndAsync()
        try {
            $bytes = [Text.UTF8Encoding]::new($false).GetBytes($raw)
            $p.StandardInput.BaseStream.Write($bytes, 0, $bytes.Length)
            $p.StandardInput.Close()
        } catch {}
        if (-not $p.WaitForExit(10000)) {
            # Whole tree: a shell's children inherit our handles and would keep Claude Code waiting.
            try { & "$env:SystemRoot\System32\taskkill.exe" /PID $p.Id /T /F 2>&1 | Out-Null } catch {}
            try { $p.Kill() } catch {}
            exit 0
        }
        [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
        [Console]::Out.Write($out.Result)
    } catch {}
    exit 0
}

# No status line of their own: a short default.
try {
    $parts = @()
    if ($status.model.display_name) { $parts += ([string]$status.model.display_name -replace '[\x00-\x1f\x7f]', '') }
    $rl = $status.rate_limits
    if ($rl.five_hour.used_percentage -ne $null) { $parts += ('5h {0:N0}%' -f [double]$rl.five_hour.used_percentage) }
    if ($rl.seven_day.used_percentage -ne $null) { $parts += ('week {0:N0}%' -f [double]$rl.seven_day.used_percentage) }
    if ($parts.Count -gt 0) { [Console]::Out.Write($parts -join ' | ') }
} catch {}
exit 0
