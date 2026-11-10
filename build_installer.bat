@echo off
setlocal
cd /d "%~dp0"

set "ISCC=%LOCALAPPDATA%\Programs\Inno Setup 6\ISCC.exe"
if not exist "%ISCC%" set "ISCC=%ProgramFiles(x86)%\Inno Setup 6\ISCC.exe"
if not exist "%ISCC%" set "ISCC=%ProgramFiles%\Inno Setup 6\ISCC.exe"
if not exist "%ISCC%" (
    echo Inno Setup 6 was not found. Install it from https://jrsoftware.org/isdl.php
    exit /b 1
)

call build_exe.bat
if errorlevel 1 exit /b 1
if not exist "dist\Rewind\Rewind.exe" (
    echo The standalone app build was not found.
    exit /b 1
)

if exist "build\InstallerStage" rmdir /s /q "build\InstallerStage"
robocopy "dist\Rewind" "build\InstallerStage" /E /NFL /NDL /NJH /NJS /NP >nul
if errorlevel 8 (
    echo Could not prepare the installer files.
    exit /b 1
)
rem FFmpeg is fetched by the installer directly from Gyan; do not bundle or redistribute it.
if exist "build\InstallerStage\ffmpeg" rmdir /s /q "build\InstallerStage\ffmpeg"

"%ISCC%" installer.iss
if errorlevel 1 exit /b 1
rmdir /s /q "build\InstallerStage"
echo.
echo Installer created: dist\Installer.exe
