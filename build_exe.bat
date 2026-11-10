@echo off
rem Builds a standalone Rewind.exe (no Python needed on the target PC).
rem Run install.bat first so FFmpeg is downloaded.
cd /d "%~dp0"
set "PY=py -3"
py -3 --version >nul 2>nul
if errorlevel 1 set "PY=python"
%PY% -m pip install -r requirements.txt pyinstaller
if errorlevel 1 goto fail
%PY% -m PyInstaller --noconfirm --clean --windowed --name Rewind --icon assets\rewind.ico ^
    --collect-all pyaudiowpatch main.py
if errorlevel 1 goto fail
if exist "ffmpeg\bin\ffmpeg.exe" (
    mkdir "dist\Rewind\ffmpeg\bin" 2>nul
    copy /y "ffmpeg\bin\ffmpeg.exe" "dist\Rewind\ffmpeg\bin\" >nul
) else (
    echo WARNING: ffmpeg\bin\ffmpeg.exe not found - run install.bat first, or copy ffmpeg.exe in yourself.
)
echo.
echo Done! Your app is in dist\Rewind\Rewind.exe  (keep the whole folder together)
pause
exit /b 0
:fail
echo Build failed.
pause
exit /b 1
