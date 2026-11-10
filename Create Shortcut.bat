@echo off
rem Puts a Rewind shortcut on your Desktop and in the Start menu.
rem While Rewind is running (it can sit in the tray), Alt + 0 opens it from anywhere.
cd /d "%~dp0"
set "PY=py -3"
py -3 --version >nul 2>nul
if errorlevel 1 set "PY=python"
%PY% -c "import sys; sys.path.insert(0, r'%~dp0.'); from app.winapi import create_shortcuts; sys.exit(0 if create_shortcuts() else 1)"
if errorlevel 1 (
    echo Couldn't create the shortcut. Make sure you ran install.bat first.
    pause
    exit /b 1
)
echo.
echo   Done! Rewind is on your Desktop and in the Start menu.
echo   While Rewind is running in the tray, press Alt + 0 to open it.
echo.
timeout /t 4 >nul
