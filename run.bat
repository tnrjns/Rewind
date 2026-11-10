@echo off
cd /d "%~dp0"
where pyw >nul 2>nul
if errorlevel 1 goto plain
start "" pyw -3 "%~dp0main.py"
exit /b 0
:plain
start "" pythonw "%~dp0main.py"
