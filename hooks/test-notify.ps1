# Tests the approval path of notify.ps1 against a throwaway LOCALAPPDATA (real app data is
# never touched). Exits 1 on the first failure. Run: powershell -File hooks/test-notify.ps1
$ErrorActionPreference = 'Stop'
$script = Join-Path $PSScriptRoot 'notify.ps1'
$root = Join-Path ([IO.Path]::GetTempPath()) ("agentpulse-hooktest-" + [guid]::NewGuid())
$data = Join-Path $root 'dev.agentpulse.app'
New-Item -ItemType Directory -Force (Join-Path $data 'decisions') | Out-Null
$hookInput = '{"hook_event_name":"PermissionRequest","session_id":"s1","cwd":"C:\\code\\demo","tool_name":"Bash","tool_input":{"command":"rm -rf build && echo \"done\""}}'
$failed = 0
$standIn = $null

function Invoke-Hook {
    $env:LOCALAPPDATA = $root
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $out = $hookInput | powershell -NoProfile -ExecutionPolicy Bypass -File $script
    [pscustomobject]@{ out = ($out -join ''); code = $LASTEXITCODE; secs = $sw.Elapsed.TotalSeconds }
}
function Set-Approvals($enabled, $wait) {
    @{ approvals = @{ enabled = $enabled; wait_seconds = $wait } } | ConvertTo-Json | Set-Content (Join-Path $data 'settings.json')
}
function Send-AnswerWhenAsked($json) {
    # Plays the app: waits for the request event, then drops an answer file (tmp + rename).
    Start-Job -ArgumentList $data, $json -ScriptBlock {
        param($data, $json)
        for ($i = 0; $i -lt 75; $i++) {
            Start-Sleep -Milliseconds 200
            $ev = Get-Content (Join-Path $data 'events.jsonl') -ErrorAction SilentlyContinue | Select-Object -Last 1 | ConvertFrom-Json
            if ($ev.event -eq 'PermissionRequest') {
                $tmp = Join-Path $data "decisions\$($ev.request_id).tmp"
                Set-Content $tmp $json -NoNewline
                Move-Item $tmp (Join-Path $data "decisions\$($ev.request_id).json"); return
            }
        }
    } | Out-Null
}
function Assert($name, $cond, $detail) {
    if ($cond) { Write-Output "ok   $name" } else { Write-Output "FAIL $name :: $detail"; $script:failed++ }
}

try {
    $appRunning = [bool](Get-Process -Name 'AgentPulse' -ErrorAction SilentlyContinue)
    if (-not $appRunning) {
        Set-Approvals $true 15
        $r = Invoke-Hook
        Assert 'app not running: no output, no wait' ($r.out -eq '' -and $r.code -eq 0 -and $r.secs -lt 10) "$($r | ConvertTo-Json -Compress)"
        # Stand-in so the hook sees "AgentPulse" running (a renamed copy of ping, sleeping).
        $exe = Join-Path $root 'AgentPulse.exe'
        Copy-Item (Join-Path $env:SystemRoot 'System32\PING.EXE') $exe
        $standIn = Start-Process $exe -ArgumentList '-n', '600', '127.0.0.1' -WindowStyle Hidden -PassThru
    }

    Set-Approvals $false 15
    $r = Invoke-Hook
    Assert 'feature off: no output' ($r.out -eq '' -and $r.code -eq 0) "$($r | ConvertTo-Json -Compress)"

    Set-Approvals $true 15
    Send-AnswerWhenAsked '{"behavior":"allow"}'
    $r = Invoke-Hook
    Assert 'allow: exact documented JSON' ($r.out -eq '{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}') $r.out

    Send-AnswerWhenAsked '{"behavior":"deny","message":"Not on main"}'
    $r = Invoke-Hook
    Assert 'deny: reason passed to Claude' ($r.out -eq '{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny","message":"Not on main"}}}') $r.out

    Send-AnswerWhenAsked '{"behavior":"allow-forever"}'
    $r = Invoke-Hook
    Assert 'unknown answer: no output (normal prompt)' ($r.out -eq '' -and $r.code -eq 0) $r.out

    $r = Invoke-Hook
    Assert 'timeout: no output after the wait' ($r.out -eq '' -and $r.code -eq 0 -and $r.secs -ge 14) "$($r | ConvertTo-Json -Compress)"

    '{ not json' | Set-Content (Join-Path $data 'settings.json')
    $r = Invoke-Hook
    Assert 'broken settings: no output' ($r.out -eq '' -and $r.code -eq 0) $r.out

    $left = @(Get-ChildItem (Join-Path $data 'decisions')).Count
    Assert 'no leftover answer files' ($left -eq 0) "$left left"
} finally {
    Get-Job | Remove-Job -Force
    if ($standIn) { Stop-Process -Id $standIn.Id -Force -ErrorAction SilentlyContinue; Start-Sleep -Milliseconds 300 }
    Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue
}
if ($failed) { Write-Output "$failed failed"; exit 1 }
Write-Output 'all hook tests passed'
