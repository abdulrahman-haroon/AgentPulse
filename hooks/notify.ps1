# AgentPulse hook for Claude Code. Register it for: SessionStart, UserPromptSubmit,
# Notification, Stop, SessionEnd (see README).
# Every event is appended to AgentPulse's local events file (that is how the Live board
# knows what each session is doing). If the app isn't running, Stop and Notification
# also show a Windows toast from here.
# Hook input arrives as JSON on stdin and is treated as untrusted data: it is re-serialized
# as JSON or placed into toast text nodes, never executed or interpolated into XML.

$ErrorActionPreference = 'Stop'
try {
    [Console]::InputEncoding = [Text.Encoding]::UTF8
    $evt = [Console]::In.ReadToEnd() | ConvertFrom-Json
    $known = 'SessionStart', 'UserPromptSubmit', 'Notification', 'Stop', 'SessionEnd'
    if ($evt.hook_event_name -notin $known) { exit 0 }
    $project = if ($evt.cwd) { Split-Path -Leaf $evt.cwd } else { 'Claude Code' }

    $dir = Join-Path $env:LOCALAPPDATA 'dev.agentpulse.app'
    [void](New-Item -ItemType Directory -Force $dir)
    $line = [ordered]@{
        event             = [string]$evt.hook_event_name
        session_id        = [string]$evt.session_id
        project           = $project
        message           = [string]$evt.message
        notification_type = [string]$evt.notification_type
        ts                = (Get-Date).ToUniversalTime().ToString('o')
    } | ConvertTo-Json -Compress
    # UTF-8 without BOM, one line per event; the app reads complete lines only.
    [IO.File]::AppendAllText((Join-Path $dir 'events.jsonl'), $line + "`n", [Text.UTF8Encoding]::new($false))

    if (Get-Process -Name 'AgentPulse' -ErrorAction SilentlyContinue) { exit 0 }

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
    # A hook must never block or fail Claude Code; swallow and exit cleanly.
}
exit 0
