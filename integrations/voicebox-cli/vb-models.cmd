@echo off
setlocal
set "VBPY=%LOCALAPPDATA%\Programs\Python\Python311\python.exe"
if not exist "%VBPY%" set "VBPY=python"
"%VBPY%" "%~dp0vbcli.py" models %*
