#ifndef AppVersion
  #define AppVersion "1.2.0"
#endif

[Setup]
AppId={{A6E2D89A-693C-4D17-AE40-8748D3152104}
AppName=Rewind
AppVersion={#AppVersion}
AppPublisher=Rewind
DefaultDirName={localappdata}\Programs\Rewind
DefaultGroupName=Rewind
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
DisableProgramGroupPage=yes
SetupIconFile=assets\rewind.ico
UninstallDisplayIcon={app}\Rewind.exe
OutputDir=dist
OutputBaseFilename=Installer
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional shortcuts:"; Flags: unchecked

[Files]
Source: "build\InstallerStage\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "install_ffmpeg.ps1"; Flags: dontcopy

[Icons]
Name: "{group}\Rewind"; Filename: "{app}\Rewind.exe"
Name: "{autodesktop}\Rewind"; Filename: "{app}\Rewind.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\Rewind.exe"; Description: "Launch Rewind"; Flags: postinstall nowait skipifsilent
Filename: "{app}\Rewind.exe"; Flags: nowait; Check: WizardSilent

[Code]
function FFmpegAvailable: Boolean;
var
  ResultCode: Integer;
begin
  Result := FileExists(ExpandConstant('{app}\ffmpeg\bin\ffmpeg.exe'));
  if not Result then
    if Exec(ExpandConstant('{sys}\where.exe'), 'ffmpeg.exe', '', SW_HIDE,
      ewWaitUntilTerminated, ResultCode) then
      Result := (ResultCode = 0);
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  ResultCode: Integer;
  PowerShell: String;
  Parameters: String;
begin
  if (CurStep = ssPostInstall) and (not FFmpegAvailable) then
  begin
    ExtractTemporaryFile('install_ffmpeg.ps1');
    WizardForm.StatusLabel.Caption := 'Downloading FFmpeg. This may take a few minutes...';
    WizardForm.Update;
    PowerShell := ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe');
    Parameters := '-NoProfile -ExecutionPolicy Bypass -File "' +
      ExpandConstant('{tmp}\install_ffmpeg.ps1') + '" -Destination "' +
      ExpandConstant('{app}') + '"';
    if not Exec(PowerShell, Parameters, '', SW_HIDE, ewWaitUntilTerminated, ResultCode) then
      RaiseException('Could not start PowerShell to install FFmpeg.');
    if ResultCode <> 0 then
      RaiseException('FFmpeg download failed. Check your internet connection and reinstall Rewind.');
  end;
end;
