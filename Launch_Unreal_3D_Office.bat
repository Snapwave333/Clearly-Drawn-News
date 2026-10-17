@echo off
setlocal
title Atlas Studio - Unreal Engine 5.8 3D Virtual Spectator View
echo =====================================================================
echo  Atlas Studio - 1-Click Virtual Spectator's View Window
echo  Engine: Unreal Engine 5.8 (DirectX 12 SM6 / Lumen / MetaHumans)
echo  Project: AtlasStudioOffice.uproject
echo  Mode: Standalone Virtual Spectator View (-game)
echo =====================================================================
cd /d "C:\Users\chrom\Desktop\Marker\AtlasStudioOffice"

rem Terminate any stale editor instance to guarantee clean spectator window launch
taskkill /F /IM UnrealEditor.exe >nul 2>&1
ping 127.0.0.1 -n 2 >nul

rem Launch Standalone Virtual Spectator Game Window directly (never open the Editor GUI)
start "" "C:\Program Files\Epic Games\UE_5.8\Engine\Binaries\Win64\UnrealEditor.exe" "C:\Users\chrom\Desktop\Marker\AtlasStudioOffice\AtlasStudioOffice.uproject" -game -windowed -ResX=1920 -ResY=1080 -WinX=60 -WinY=30 -messaging
exit /b 0
