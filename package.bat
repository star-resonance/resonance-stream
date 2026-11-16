@echo off
setlocal enabledelayedexpansion

:: --- Configuration ---
set APP_NAME=Resonance-Stream
set OUTPUT_DIR=dist

:: [1/2] Build the in-game DLL (the only build path: it links the C runtime statically
:: and fails on any C-runtime import; needs `just`)
echo [1/2] Building the in-game DLL...
call just ingame-build
if errorlevel 1 (
    echo ERROR: the in-game DLL build failed.
    pause
    exit /b 1
)

:: The installer bundles that DLL: the overlay config maps it, and the app's build script
:: requires it at this path.
set RESONANCE_INGAME_DLL=%CD%\target\x86_64-pc-windows-msvc\ingame\resonance_ingame.dll
set RESONANCE_INGAME_REQUIRED=1

:: [2/2] Build Tauri Installer
echo [2/2] Building Windows Installer...
call cargo tauri build --config src-tauri\tauri.ingame.conf.json
if errorlevel 1 (
    echo ERROR: the installer build failed.
    pause
    exit /b 1
)

:: Finalizing Release
echo Finalizing Release...
if not exist "%OUTPUT_DIR%" mkdir "%OUTPUT_DIR%"

set FOUND_INSTALLER=0
for /r "src-tauri\target\release\bundle\nsis" %%f in (*-setup.exe) do (
    echo Found: %%f
    move /Y "%%f" "%OUTPUT_DIR%\"
    set FOUND_INSTALLER=1
)

echo.
echo ========================================================
echo  Build Complete!
echo  Installer is ready in: %CD%\%OUTPUT_DIR%
echo ========================================================
pause