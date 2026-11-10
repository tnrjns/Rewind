@echo off
setlocal EnableExtensions
cd /d "%~dp0"
title Rewind setup
echo.
echo   ===========================
echo      Rewind  -  setup
echo   ===========================
echo.

rem ---- find Python 3.10+
set "PY=py -3"
py -3 -c "import sys; sys.exit(0 if sys.version_info >= (3,10) else 1)" >nul 2>nul
if not errorlevel 1 goto havepy
set "PY=python"
python -c "import sys; sys.exit(0 if sys.version_info >= (3,10) else 1)" >nul 2>nul
if not errorlevel 1 goto havepy
echo Python 3.10 or newer was not found. Installing Python 3.12 with winget...
winget install -e --id Python.Python.3.12 --accept-package-agreements --accept-source-agreements
echo.
echo Python installed. Close this window and run install.bat again.
pause
exit /b 1

:havepy
echo [1/3] Using Python:
%PY% --version

echo.
echo [2/3] Installing Python packages (PySide6, PyAudioWPatch)...
%PY% -m pip install --upgrade pip >nul
%PY% -m pip install -r requirements.txt
if errorlevel 1 goto fail

echo.
if exist "ffmpeg\bin\ffmpeg.exe" (
    echo [3/3] FFmpeg already present.
    goto shortcut
)
echo [3/3] Downloading FFmpeg (about 90 MB, one time only)...
powershell -NoProfile -ExecutionPolicy Bypass -Command ^
  "$ErrorActionPreference='Stop'; $ProgressPreference='SilentlyContinue';" ^
  "$zip = Join-Path $env:TEMP 'rewind-ffmpeg.zip'; $tmp = Join-Path $env:TEMP 'rewind-ffmpeg';" ^
  "Invoke-WebRequest -Uri 'https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip' -OutFile $zip -UseBasicParsing;" ^
  "if (Test-Path $tmp) { Remove-Item $tmp -Recurse -Force };" ^
  "Expand-Archive -Path $zip -DestinationPath $tmp -Force;" ^
  "$dir = Get-ChildItem $tmp -Directory | Select-Object -First 1;" ^
  "if (Test-Path 'ffmpeg') { Remove-Item 'ffmpeg' -Recurse -Force };" ^
  "Move-Item $dir.FullName 'ffmpeg'; Remove-Item $zip -Force"
if not exist "ffmpeg\bin\ffmpeg.exe" (
    echo.
    echo Couldn't download FFmpeg automatically. Trying winget instead...
    winget install -e --id Gyan.FFmpeg --accept-package-agreements --accept-source-agreements
)

:shortcut
echo.
echo Creating Desktop and Start menu shortcuts ...
for /f "delims=" %%i in ('%PY% -c "import sys,os;print(os.path.join(os.path.dirname(sys.executable),'pythonw.exe'))"') do set "PYW=%%i"
%PY% -c "import sys; sys.path.insert(0, r'%~dp0.'); from app.winapi import create_shortcuts; sys.exit(0 if create_shortcuts() else 1)"
if errorlevel 1 echo   Couldn't create the shortcut - you can use run.bat or Create Shortcut.bat instead.

echo.
echo   All done! Launching Rewind...
start "" "%PYW%" "%~dp0main.py"
timeout /t 3 >nul
exit /b 0

:fail
echo.
echo Setup failed. Scroll up to see what went wrong.
pause
exit /b 1
