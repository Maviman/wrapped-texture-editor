; Inno Setup 6 script for the Wrapped Texture Editor installer (per user, no admin rights).
;
; Built by .github/workflows/installer.yml; by hand:
;   ISCC /DAppVersion=0.1.0 /DAppVersionNumeric=0.1.0.0 /DStageDir=<staged files> /DOutputDir=<out> installer\WrappedTextureEditor.iss
; StageDir holds the files under their installed names (see the workflow's "Stage" step):
;   Wrapped Texture Editor.exe, app.ico, Start - DirectX 12.cmd, Start - Vulkan without overlays.cmd, Examples\*
;
; Silent use:  Setup.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART [/COMPONENTS=...] [/TASKS=...] [/LOG=file]
; Uninstall:   unins000.exe /VERYSILENT [/PURGEUSERDATA]   (without /PURGEUSERDATA a silent uninstall keeps user data)

#ifndef AppVersion
  #define AppVersion "0.0.0-dev"
#endif
#ifndef AppVersionNumeric
  #define AppVersionNumeric "0.0.0.0"
#endif
#ifndef StageDir
  #define StageDir "..\target\installer\stage"
#endif
#ifndef OutputDir
  #define OutputDir "..\target\installer"
#endif
#ifndef OutputBaseFilename
  #define OutputBaseFilename "WrappedTextureEditor-" + AppVersion + "-Setup"
#endif

#define AppName "Wrapped Texture Editor"
#define AppExeName "Wrapped Texture Editor.exe"
#define RepoUrl "https://github.com/Maviman/wrapped-texture-editor"
#define ProjectProgId "WrappedTextureEditor.Project"

[Setup]
; Never change AppId: it is what makes a newer Setup.exe upgrade the existing install in place.
AppId={{C73E1B7D-2702-4D07-9460-D71FB97FD2B4}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=Maviman
AppPublisherURL={#RepoUrl}
AppSupportURL={#RepoUrl}/issues
AppUpdatesURL={#RepoUrl}/releases
VersionInfoVersion={#AppVersionNumeric}
VersionInfoProductVersion={#AppVersionNumeric}
VersionInfoDescription={#AppName} Setup
; Per user: installs into %LOCALAPPDATA%\Programs, writes only HKCU, never asks for admin rights.
PrivilegesRequired=lowest
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; Windows 10 1809 or later (DirectX 12 and current Vulkan drivers).
MinVersion=10.0.17763
OutputDir={#OutputDir}
OutputBaseFilename={#OutputBaseFilename}
SetupIconFile=app.ico
UninstallDisplayIcon={app}\app.ico
UninstallDisplayName={#AppName}
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
; Upgrades close a running copy through the Restart Manager instead of failing on a locked exe.
CloseApplications=yes
RestartApplications=no
ChangesAssociations=yes
SetupLogging=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Types]
Name: "full"; Description: "Full installation"
Name: "compact"; Description: "Editor only"
Name: "custom"; Description: "Custom installation"; Flags: iscustom

[Components]
Name: "main"; Description: "Wrapped Texture Editor"; Types: full compact custom; Flags: fixed
Name: "gpulaunchers"; Description: "Graphics troubleshooting start options (DirectX 12; Vulkan without overlays)"; Types: full
Name: "examples"; Description: "Example models (small OBJ and glTF shapes)"; Types: full

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: unchecked
Name: "assoc"; Description: "Open .wtep &project files with {#AppName}"; GroupDescription: "File types:"

[Files]
Source: "{#StageDir}\{#AppExeName}"; DestDir: "{app}"; Flags: ignoreversion; Components: main
Source: "{#StageDir}\app.ico"; DestDir: "{app}"; Flags: ignoreversion; Components: main
Source: "{#StageDir}\Start - DirectX 12.cmd"; DestDir: "{app}"; Flags: ignoreversion; Components: gpulaunchers
Source: "{#StageDir}\Start - Vulkan without overlays.cmd"; DestDir: "{app}"; Flags: ignoreversion; Components: gpulaunchers
Source: "{#StageDir}\Examples\*"; DestDir: "{app}\Examples"; Flags: ignoreversion recursesubdirs; Components: examples

[InstallDelete]
; A component unticked during an upgrade must not leave its old files behind.
Type: files; Name: "{app}\Start - DirectX 12.cmd"; Components: not gpulaunchers
Type: files; Name: "{app}\Start - Vulkan without overlays.cmd"; Components: not gpulaunchers
Type: filesandordirs; Name: "{app}\Examples"; Components: not examples

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExeName}"; IconFilename: "{app}\app.ico"
Name: "{group}\{#AppName} (DirectX 12)"; Filename: "{app}\Start - DirectX 12.cmd"; WorkingDir: "{app}"; IconFilename: "{app}\app.ico"; Flags: runminimized; Components: gpulaunchers
Name: "{group}\{#AppName} (Vulkan without overlays)"; Filename: "{app}\Start - Vulkan without overlays.cmd"; WorkingDir: "{app}"; IconFilename: "{app}\app.ico"; Flags: runminimized; Components: gpulaunchers
Name: "{group}\Example models"; Filename: "{app}\Examples"; Components: examples
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExeName}"; IconFilename: "{app}\app.ico"; Tasks: desktopicon

[Registry]
; .wtep association, per user (HKA = HKCU for a per-user install). Removed again by the uninstaller.
Root: HKA; Subkey: "Software\Classes\.wtep"; ValueType: string; ValueName: ""; ValueData: "{#ProjectProgId}"; Flags: uninsdeletevalue; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\.wtep\OpenWithProgids"; ValueType: string; ValueName: "{#ProjectProgId}"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\{#ProjectProgId}"; ValueType: string; ValueName: ""; ValueData: "{#AppName} project"; Flags: uninsdeletekey; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\{#ProjectProgId}\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\app.ico"; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\{#ProjectProgId}\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExeName}"" ""%1"""; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExeName}\SupportedTypes"; ValueType: string; ValueName: ".wtep"; ValueData: ""; Flags: uninsdeletekey; Tasks: assoc

[Run]
Filename: "{app}\{#AppExeName}"; Description: "Start {#AppName} now"; Flags: nowait postinstall skipifsilent

[Code]
const
  GpuOk = 0;       { gpu_check.rs EXIT_OK: a hardware Vulkan or DirectX 12 adapter }
  GpuLimited = 2;  { only software or OpenGL adapters }
  GpuNone = 3;     { no graphics adapter at all }

var
  SystemPage: TOutputMsgMemoWizardPage;
  GpuCode: Integer;
  GpuChecked: Boolean;
  GpuSummary: String;

{ Value of `key=` in the report lines, '' when missing. }
function ReportValue(Lines: TArrayOfString; Key: String): String;
var
  I: Integer;
begin
  Result := '';
  for I := 0 to GetArrayLength(Lines) - 1 do
    if Pos(Key + '=', Lines[I]) = 1 then
    begin
      Result := Copy(Lines[I], Length(Key) + 2, MaxInt);
      Exit;
    end;
end;

function DriverAdvice(): String;
begin
  Result :=
    'Install the latest graphics driver from your GPU maker, then start the editor:' + #13#10 +
    '  NVIDIA: https://www.nvidia.com/Download/index.aspx' + #13#10 +
    '  AMD: https://www.amd.com/en/support' + #13#10 +
    '  Intel: https://www.intel.com/content/www/us/en/download-center/home.html' + #13#10 +
    '  (or Windows Update > Advanced options > Optional updates > Driver updates)';
end;

{ Where the DirectX 12 / Vulkan-without-overlays start options are, or how to get them. }
function TroubleshootAdvice(): String;
begin
  if WizardIsComponentSelected('gpulaunchers') then
    Result := 'If the editor then does not start, use its "DirectX 12" or "Vulkan without overlays" shortcut in the Start menu.'
  else
    Result := 'If the editor then does not start, run Setup again and tick "Graphics troubleshooting start options".';
end;

{ Runs the editor's own --gpu-check (no window, no focus change) from the setup's temp folder and fills GpuCode and
  GpuSummary. Runs once; also in silent mode, where the result only goes to the setup log. }
procedure RunGpuCheck();
var
  Exe, Report, Best: String;
  Lines: TArrayOfString;
  Code: Integer;
begin
  if GpuChecked then
    Exit;
  GpuChecked := True;
  ExtractTemporaryFile('{#AppExeName}');
  Exe := ExpandConstant('{tmp}\{#AppExeName}');
  Report := ExpandConstant('{tmp}\gpu-check.txt');
  if not Exec(Exe, '--gpu-check "' + Report + '"', ExpandConstant('{tmp}'), SW_HIDE, ewWaitUntilTerminated, Code) then
    Code := -1;
  GpuCode := Code;
  if not LoadStringsFromFile(Report, Lines) then
    SetArrayLength(Lines, 0);
  Best := ReportValue(Lines, 'best');

  GpuSummary := 'Windows ' + GetWindowsVersionString + ', 64-bit: OK' + #13#10 +
    'Microsoft Visual C++ runtime: not needed (the editor is self-contained)' + #13#10 +
    'Vulkan driver: ' + ReportValue(Lines, 'vulkan') + #13#10 +
    'DirectX 12: ' + ReportValue(Lines, 'dx12') + #13#10#13#10;
  case Code of
    GpuOk:
      GpuSummary := GpuSummary + 'Graphics: OK - the editor will use ' + Best + '.';
    GpuLimited:
      GpuSummary := GpuSummary +
        'WARNING: only a software or OpenGL renderer was found (' + Best + ').' + #13#10 +
        'The editor will start, but it can be slow and some views may not work.' + #13#10#13#10 + DriverAdvice();
    GpuNone:
      GpuSummary := GpuSummary +
        'PROBLEM: no graphics driver with Vulkan or DirectX 12 was found.' + #13#10 +
        'The editor needs one and will not start without it. You can still install it now.' + #13#10#13#10 + DriverAdvice();
  else
    GpuSummary := GpuSummary +
      'WARNING: the graphics check could not finish (code ' + IntToStr(Code) + ': ' + ReportValue(Lines, 'error') + ').' + #13#10 +
      'This usually means a broken graphics driver or an overlay program (OBS, Steam, Overwolf) that hooks into it.' + #13#10#13#10 +
      DriverAdvice() + #13#10 + TroubleshootAdvice();
  end;
  Log('Graphics check exit code ' + IntToStr(Code) + #13#10 + GpuSummary);
end;

procedure InitializeWizard();
begin
  SystemPage := CreateOutputMsgMemoPage(wpSelectTasks,
    'System check', 'Does this computer have what the editor needs?',
    'Setup checked Windows and the graphics driver. Nothing has been changed on your computer yet.', '');
end;

procedure CurPageChanged(CurPageID: Integer);
begin
  if CurPageID = SystemPage.ID then
  begin
    RunGpuCheck();
    SystemPage.RichEditViewer.Lines.Text := GpuSummary;
  end;
end;

function NextButtonClick(CurPageID: Integer): Boolean;
begin
  Result := True;
  { Never in a silent install: nobody could answer, and the result is already in the setup log. }
  if (CurPageID = SystemPage.ID) and (GpuCode = GpuNone) and not WizardSilent then
    Result := MsgBox('No graphics driver with Vulkan or DirectX 12 was found, so the editor will not start until one is ' +
      'installed.' + #13#10#13#10 + 'Install the editor anyway?', mbConfirmation, MB_YESNO) = IDYES;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  { Silent installs skip the wizard pages: still run the check so the setup log records it. }
  if CurStep = ssInstall then
    RunGpuCheck();
end;

{ True when the user data should go too: /PURGEUSERDATA on the command line, or Yes in the uninstall question. }
function PurgeUserData(): Boolean;
var
  I: Integer;
begin
  Result := False;
  for I := 1 to ParamCount do
    if CompareText(ParamStr(I), '/PURGEUSERDATA') = 0 then
    begin
      Result := True;
      Exit;
    end;
  if not UninstallSilent then
    Result := MsgBox('Also delete your editor settings, logs and crash reports?' + #13#10#13#10 +
      ExpandConstant('{userappdata}\WrappedTextureEditor') + #13#10 + ExpandConstant('{app}\crash') + #13#10#13#10 +
      'Your .wtep projects and exported textures are never deleted. Choose No to keep the settings for a later install.',
      mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES;
end;

{ The uninstaller cannot remove a running editor (a per-user uninstaller cannot delete on reboot), which would leave an
  orphaned exe without an Add/Remove Programs entry: ask to close it first. A silent uninstall stops with an error. }
function InitializeUninstall(): Boolean;
var
  Exe: String;
begin
  Result := True;
  Exe := ExpandConstant('{app}\{#AppExeName}');
  while FileExists(Exe) and not RenameFile(Exe, Exe + '.uninstall-check') do
    if SuppressibleMsgBox('Wrapped Texture Editor is still running. Close it (save your work first), then click Retry.',
      mbError, MB_RETRYCANCEL, IDCANCEL) <> IDRETRY then
    begin
      Result := False;
      Exit;
    end;
  if FileExists(Exe + '.uninstall-check') then
    RenameFile(Exe + '.uninstall-check', Exe);
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  DataDir, CrashDir: String;
begin
  if CurUninstallStep = usUninstall then
  begin
    DataDir := ExpandConstant('{userappdata}\WrappedTextureEditor');
    CrashDir := ExpandConstant('{app}\crash');
    if (DirExists(DataDir) or DirExists(CrashDir)) and PurgeUserData() then
    begin
      DelTree(DataDir, True, True, True);
      DelTree(CrashDir, True, True, True);
      Log('User data deleted on request');
    end
    else
      Log('User data kept');
  end;
end;
