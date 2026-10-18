@echo off
rem Clearly Drawn News daemon: news slots only (no explainer publishing).
rem Runs from Marker\ so youtube_credentials.json and youtube_token.json resolve.
cd /d "%~dp0..\.."
set "LOGDIR=%LOCALAPPDATA%\atlas-studio\atlas\data\daemon"
if not exist "%LOGDIR%" mkdir "%LOGDIR%"
"%CD%\atlas-studio\target\release\atlas.exe" daemon start --news-only >> "%LOGDIR%\daemon.log" 2>&1
