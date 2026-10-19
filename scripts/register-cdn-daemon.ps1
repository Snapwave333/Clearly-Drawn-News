<#
Registers (or re-registers) the Task Scheduler task "CDN News Daemon".
Default: DISABLED. Publishing stays governed by ATLAS_NEWS_PUBLISH (default off) regardless.

  powershell -ExecutionPolicy Bypass -File scripts\register-cdn-daemon.ps1           # register/update, disabled
  powershell -ExecutionPolicy Bypass -File scripts\register-cdn-daemon.ps1 -Enable   # go-live only, when Eli says go

Self-healing:
  * Trigger: at logon of the current user (survives reboots; enable Windows auto-logon for unattended boots).
  * The action runs scripts\cdn-news-daemon.cmd, a supervisor loop that restarts atlas.exe after any crash.
  * If the supervisor itself dies, Task Scheduler restarts it every 1 minute, up to 999 times.
  * StartWhenAvailable runs a missed start; no execution time limit; battery does not stop it.
  * MultipleInstances IgnoreNew plus the daemon's loopback lock keep a single instance.
#>
param([switch]$Enable)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$marker = Split-Path -Parent $repo
$launcher = Join-Path $repo 'scripts\cdn-news-daemon.cmd'
$name = 'CDN News Daemon'
$action = New-ScheduledTaskAction -Execute "$env:WINDIR\System32\cmd.exe" -Argument "/c `"$launcher`"" -WorkingDirectory $marker
$trigger = New-ScheduledTaskTrigger -AtLogOn -User "$env:USERDOMAIN\$env:USERNAME"
$settings = New-ScheduledTaskSettingsSet -RestartCount 999 -RestartInterval (New-TimeSpan -Minutes 1) `
  -ExecutionTimeLimit ([TimeSpan]::Zero) -StartWhenAvailable -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries `
  -MultipleInstances IgnoreNew
$principal = New-ScheduledTaskPrincipal -UserId "$env:USERDOMAIN\$env:USERNAME" -LogonType Interactive -RunLevel Limited
$task = New-ScheduledTask -Action $action -Trigger $trigger -Settings $settings -Principal $principal `
  -Description 'Clearly Drawn News daemon (atlas daemon start --news-only) via self-healing launcher. Publishing is controlled by ATLAS_NEWS_PUBLISH.'
Register-ScheduledTask -TaskName $name -InputObject $task -Force | Out-Null
if ($Enable) { Enable-ScheduledTask -TaskName $name | Out-Null } else { Disable-ScheduledTask -TaskName $name | Out-Null }
$t = Get-ScheduledTask -TaskName $name
"{0}: {1}; restart {2}x every {3}; launcher {4}" -f $name, $t.State, $t.Settings.RestartCount, $t.Settings.RestartInterval, $launcher
