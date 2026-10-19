@echo off
rem Clearly Drawn News daemon supervisor: news slots only (no explainer publishing).
rem Runs from Marker\ so youtube_credentials.json and youtube_token.json resolve.
rem Self-heals: restarts atlas.exe after any exit with a growing back-off (1 min x failures, max 15 min),
rem rotates daemon.log at 20 MB, sends a best-effort Telegram notice (atlas daemon notify) on each crash,
rem and stops for good after `atlas daemon pause` (daemon\paused) or when another daemon holds the lock (exit 3).
rem Task Scheduler task "CDN News Daemon" runs this at logon (registered DISABLED; see scripts\register-cdn-daemon.ps1).
setlocal EnableDelayedExpansion
cd /d "%~dp0..\.."
set "LOGDIR=%LOCALAPPDATA%\atlas-studio\atlas\data\daemon"
set "LOG=%LOGDIR%\daemon.log"
set "ATLAS=%CD%\atlas-studio\target\release\atlas.exe"
if not exist "%LOGDIR%" mkdir "%LOGDIR%"
set /a FAILS=0

:loop
for %%F in ("%LOG%") do if %%~zF GTR 20000000 move /y "%LOG%" "%LOG%.1" >nul
echo [%date% %time%] launcher: starting atlas daemon start --news-only>> "%LOG%"
"%ATLAS%" daemon start --news-only >> "%LOG%" 2>&1
set "CODE=!errorlevel!"
if exist "%LOGDIR%\paused" (
  echo [%date% %time%] launcher: daemon paused; supervisor stopping.>> "%LOG%"
  exit /b 0
)
if "!CODE!"=="3" (
  echo [%date% %time%] launcher: another daemon holds the lock; supervisor stopping.>> "%LOG%"
  exit /b 0
)
set /a FAILS+=1
set /a DELAY=60*FAILS
if !DELAY! GTR 900 set /a DELAY=900
echo [%date% %time%] launcher: daemon exited with code !CODE!; restart !FAILS! in !DELAY! s.>> "%LOG%"
if exist "%ATLAS%" "%ATLAS%" daemon notify --message "CDN News daemon exited with code !CODE!; restart !FAILS! in !DELAY! s. Log: %LOG%" >> "%LOG%" 2>&1
timeout /t !DELAY! /nobreak >nul
goto loop
