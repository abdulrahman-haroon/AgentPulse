# AgentPulse hook for Claude Code. Registered (by Setup > Connect hooks) for: SessionStart,
# UserPromptSubmit, Notification, PermissionRequest, Stop, SessionEnd.
# Every event is appended to AgentPulse's local events file (that is how the Live board
# knows what each session is doing). If the app isn't running, Stop and Notification
# also show a Windows toast from here.
#
# PermissionRequest (only when "Approve in AgentPulse" is on AND the app is running): the
# request is handed to the app, and this script waits for the user's Allow/Deny there.
# Anything else (off, app closed, timeout, bad answer, any error) prints nothing and exits 0,
# so Claude Code shows its normal permission prompt. Nothing is ever approved by default.
#
# Hook input is untrusted: it is re-serialized as JSON or placed into toast text nodes,
# never executed or interpolated into XML. Keep this file ASCII (PowerShell 5.1 reads it as ANSI).

$ErrorActionPreference = 'Stop'
$dir = Join-Path $env:LOCALAPPDATA 'dev.agentpulse.app'

function Add-AgentPulseEvent([System.Collections.IDictionary]$fields) {
    $fields['ts'] = (Get-Date).ToUniversalTime().ToString('o')
    $line = $fields | ConvertTo-Json -Compress -Depth 4
    # UTF-8 without BOM, one line per event; the app reads complete lines only.
    [IO.File]::AppendAllText((Join-Path $dir 'events.jsonl'), $line + "`n", [Text.UTF8Encoding]::new($false))
}

function Test-AgentPulseRunning { [bool](Get-Process -Name 'AgentPulse' -ErrorAction SilentlyContinue) }

try {
    [Console]::InputEncoding = [Text.Encoding]::UTF8
    $evt = [Console]::In.ReadToEnd() | ConvertFrom-Json
    $known = 'SessionStart', 'UserPromptSubmit', 'Notification', 'PermissionRequest', 'Stop', 'SessionEnd'
    if ($evt.hook_event_name -notin $known) { exit 0 }
    $project = if ($evt.cwd) { Split-Path -Leaf $evt.cwd } else { 'Claude Code' }
    [void](New-Item -ItemType Directory -Force $dir)

    if ($evt.hook_event_name -eq 'PermissionRequest') {
        $settingsFile = Join-Path $dir 'settings.json'
        if (-not (Test-Path $settingsFile) -or -not (Test-AgentPulseRunning)) { exit 0 }
        $approvals = (Get-Content $settingsFile -Raw | ConvertFrom-Json).approvals
        if (-not $approvals -or $approvals.enabled -ne $true) { exit 0 }
        $wait = [Math]::Min([Math]::Max([int]$approvals.wait_seconds, 15), 300)

        $toolInput = $evt.tool_input | ConvertTo-Json -Compress -Depth 10
        if ($toolInput.Length -gt 20000) { exit 0 }  # too big to review safely in the app; use the normal prompt

        $id = [guid]::NewGuid().ToString()
        Add-AgentPulseEvent ([ordered]@{
            event = 'PermissionRequest'; session_id = [string]$evt.session_id; project = $project
            request_id = $id; tool_name = [string]$evt.tool_name; tool_input = $toolInput; wait = $wait
        })

        $decisionFile = Join-Path $dir "decisions\$id.json"
        $deadline = (Get-Date).AddSeconds($wait)
        while ((Get-Date) -lt $deadline) {
            if (Test-Path $decisionFile) {
                $d = Get-Content $decisionFile -Raw | ConvertFrom-Json
                Remove-Item $decisionFile -Force
                $decision = $null
                if ($d.behavior -eq 'allow') {
                    $decision = [ordered]@{ behavior = 'allow' }
                } elseif ($d.behavior -eq 'deny') {
                    $decision = [ordered]@{ behavior = 'deny'; message = ([string]$d.message) }
                }
                if ($decision) {
                    Add-AgentPulseEvent ([ordered]@{ event = 'PermissionResolved'; session_id = [string]$evt.session_id; request_id = $id })
                    $out = [ordered]@{ hookSpecificOutput = [ordered]@{ hookEventName = 'PermissionRequest'; decision = $decision } }
                    [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
                    [Console]::Out.Write(($out | ConvertTo-Json -Compress -Depth 5))
                    exit 0
                }
                break  # unknown answer: fall back to the normal prompt
            }
            Start-Sleep -Milliseconds 300
        }
        Add-AgentPulseEvent ([ordered]@{ event = 'PermissionTimeout'; session_id = [string]$evt.session_id; request_id = $id })
        exit 0
    }

    Add-AgentPulseEvent ([ordered]@{
        event = [string]$evt.hook_event_name; session_id = [string]$evt.session_id; project = $project
        message = [string]$evt.message; notification_type = [string]$evt.notification_type
    })

    if (Test-AgentPulseRunning) { exit 0 }

    switch ($evt.hook_event_name) {
        'Stop'         { $title = "Claude finished: $project"; $body = 'Task complete.' }
        'Notification' { $title = "Claude needs you: $project"; $body = [string]$evt.message }
        default        { exit 0 }
    }
    if ($body.Length -gt 200) { $body = $body.Substring(0, 200) + '...' }

    [void][Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime]
    $xml = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent(
        [Windows.UI.Notifications.ToastTemplateType]::ToastText02)
    $lines = $xml.GetElementsByTagName('text')
    [void]$lines.Item(0).AppendChild($xml.CreateTextNode($title))
    [void]$lines.Item(1).AppendChild($xml.CreateTextNode($body))

    # shortcut: borrows PowerShell's registered app id, so fallback toasts say "Windows PowerShell"; run the app for branded ones.
    $appId = '{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe'
    [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier($appId).Show(
        [Windows.UI.Notifications.ToastNotification]::new($xml))
} catch {
    # A hook must never block or fail Claude Code; swallow and exit cleanly (no output = normal prompt).
}
exit 0
