# Tests statusline.ps1 against a throwaway LOCALAPPDATA (real app data is never touched).
# Exits 1 on any failure. Run: powershell -File hooks/test-statusline.ps1
$ErrorActionPreference = 'Stop'
$root = Join-Path ([IO.Path]::GetTempPath()) ("agentpulse-sltest-" + [guid]::NewGuid())
$data = Join-Path $root 'dev.agentpulse.app'
$hooks = Join-Path $data 'hooks'
New-Item -ItemType Directory -Force $hooks | Out-Null
Copy-Item (Join-Path $PSScriptRoot 'statusline.ps1') $hooks
$script = Join-Path $hooks 'statusline.ps1'
$saved = Join-Path $hooks 'statusline-original.json'
$snapFile = Join-Path $data 'claude-limits.json'
$statusJson = '{"model":{"display_name":"Opus"},"rate_limits":{"five_hour":{"used_percentage":23.6,"resets_at":1781524700},"seven_day":{"used_percentage":"41","resets_at":1781956800},"extra":{"used_percentage":1}}}'
$failed = 0

function Invoke-Line($text) {
    $env:LOCALAPPDATA = $root
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $out = $text | powershell -NoProfile -ExecutionPolicy Bypass -File $script
    [pscustomobject]@{ out = ($out -join "`n"); code = $LASTEXITCODE; secs = $sw.Elapsed.TotalSeconds }
}
function Save-Original($command) {
    $json = if ($null -eq $command) { 'null' } else { @{ type = 'command'; command = $command } | ConvertTo-Json -Compress }
    [IO.File]::WriteAllText($saved, $json, [Text.UTF8Encoding]::new($false))
}
function Assert($name, $cond, $detail) {
    if ($cond) { Write-Output "ok   $name" } else { Write-Output "FAIL $name :: $detail"; $script:failed++ }
}

try {
    $r = Invoke-Line $statusJson
    Assert 'no own line: default text' ($r.out -eq 'Opus | 5h 24% | week 41%' -and $r.code -eq 0) "$($r | ConvertTo-Json -Compress)"
    $snap = Get-Content $snapFile -Raw | ConvertFrom-Json
    Assert 'snapshot: numbers only, known windows only' ($snap.rate_limits.five_hour.used_percentage -eq 23.6 -and $snap.rate_limits.seven_day.used_percentage -eq 41 -and $snap.rate_limits.five_hour.resets_at -eq 1781524700 -and -not $snap.rate_limits.extra -and $snap.ts) (Get-Content $snapFile -Raw)

    # The user's own line works in both Git Bash and PowerShell (how Claude Code may run it).
    $line = Join-Path $root 'line.ps1'
    Set-Content $line '$s = [Console]::In.ReadToEnd() | ConvertFrom-Json; [Console]::Out.Write("MINE " + $s.model.display_name + "`nsecond line")'
    Save-Original ('powershell -NoProfile -ExecutionPolicy Bypass -File "' + ($line -replace '\\', '/') + '"')
    $r = Invoke-Line $statusJson
    Assert 'own line: runs with the same input, output passed through' ($r.out -eq "MINE Opus`nsecond line" -and $r.code -eq 0) "$($r | ConvertTo-Json -Compress)"

    Save-Original $null
    $r = Invoke-Line '{"model":{"display_name":"Sonnet"}}'
    Assert 'saved null: default text, no limits' ($r.out -eq 'Sonnet' -and $r.code -eq 0) $r.out

    Remove-Item $snapFile
    $r = Invoke-Line '{ not json'
    Assert 'bad input: no output, no snapshot, exit 0' ($r.out -eq '' -and $r.code -eq 0 -and -not (Test-Path $snapFile)) "$($r | ConvertTo-Json -Compress)"

    Save-Original 'powershell -NoProfile -Command "Start-Sleep 60"'
    $r = Invoke-Line $statusJson
    Assert 'hanging own line: killed after ~10 s' ($r.code -eq 0 -and $r.secs -lt 25) "$($r | ConvertTo-Json -Compress)"
} finally {
    Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue
}
if ($failed) { Write-Output "$failed failed"; exit 1 }
Write-Output 'all status-line tests passed'
