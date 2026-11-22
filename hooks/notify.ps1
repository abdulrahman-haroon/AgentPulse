# AgentPulse notifier: a Claude Code hook for task finished (Stop) and needs input (Notification).
# If the AgentPulse app is running, the event is appended to its events file and the app
# shows the notification. Otherwise this script shows a Windows toast itself.
# Hook input arrives as JSON on stdin and is treated as untrusted data: it is re-serialized
# as JSON or placed into toast text nodes, never executed or interpolated into XML.

$ErrorActionPreference = 'Stop'
try {
    [Console]::InputEncoding = [Text.Encoding]::UTF8
    $evt = [Console]::In.ReadToEnd() | ConvertFrom-Json
    if ($evt.hook_event_name -notin 'Stop', 'Notification') { exit 0 }
    $project = if ($evt.cwd) { Split-Path -Leaf $evt.cwd } else { 'Claude Code' }

    if (Get-Process -Name 'AgentPulse' -ErrorAction SilentlyContinue) {
        $dir = Join-Path $env:LOCALAPPDATA 'dev.agentpulse.app'
        [void](New-Item -ItemType Directory -Force $dir)
        $line = [ordered]@{ event = $evt.hook_event_name; project = $project; message = [string]$evt.message } | ConvertTo-Json -Compress
        # UTF-8 without BOM, one line per event; the app reads complete lines only.
        [IO.File]::AppendAllText((Join-Path $dir 'events.jsonl'), $line + "`n", [Text.UTF8Encoding]::new($false))
        exit 0
    }

    switch ($evt.hook_event_name) {
        'Stop'         { $title = "Claude finished: $project"; $body = 'Task complete.' }
        'Notification' { $title = "Claude needs you: $project"; $body = [string]$evt.message }
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
    # A notifier must never block or fail Claude Code; swallow and exit cleanly.
}
exit 0
