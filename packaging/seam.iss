; The GitHub installer. It installs for the current user only, with no admin prompt.
; Updates don't run it: Seam's own updater swaps the exes. See packaging/README.md.
;
;   ISCC.exe /DAppVersion=0.5.0 /DAppId={GUID from installer.json} /DSourceDir=<abs path to dist> /Odist packaging\seam.iss
;
; SourceDir holds the exes the release was built from, never target\. Give it as
; an absolute path, because Inno reads a relative one from this file's folder.

#ifndef AppVersion
  #error Pass /DAppVersion=x.y.z
#endif
#ifndef AppId
  #error Pass /DAppId= with the app_id from packaging\installer.json
#endif
#ifndef SourceDir
  #error Pass /DSourceDir= with the folder that holds seam.exe and seam-settings.exe
#endif

; Inno wants "{{" for a literal brace in AppId.
#define AppIdEscaped StringChange(AppId, "{", "{{")
; The four-part form for the file properties, without a pre-release tag.
#define Dash Pos("-", AppVersion)
#if Dash
  #define Core Copy(AppVersion, 1, Dash - 1)
#else
  #define Core AppVersion
#endif
#define Repo "https://github.com/F3NN3X/seam"

[Setup]
AppId={#AppIdEscaped}
AppName=Seam
AppVersion={#AppVersion}
AppPublisher=F3NN3X
AppPublisherURL={#Repo}
AppSupportURL={#Repo}/issues
VersionInfoVersion={#Core}.0
DefaultDirName={autopf}\Seam
DisableDirPage=yes
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
WizardStyle=modern
SetupIconFile=..\assets\seam.ico
UninstallDisplayIcon={app}\seam.exe
LicenseFile=..\LICENSE
OutputBaseFilename=Seam-Setup-{#AppVersion}
Compression=lzma2
SolidCompression=yes
; Seam is quit through its own API (see [Code]), not by Restart Manager.
CloseApplications=no
RestartApplications=no

[Files]
; Near the top so solid compression can extract it quickly. Only for PrepareToInstall.
Source: "{#SourceDir}\seam-settings.exe"; DestDir: "{tmp}"; Flags: dontcopy
Source: "{#SourceDir}\seam.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\seam-settings.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Seam"; Filename: "{app}\seam.exe"

[Run]
Filename: "{app}\seam.exe"; Description: "Start Seam"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; The updater leaves .old files behind, and keeps what it downloaded here.
Type: files; Name: "{app}\*.old"
Type: filesandordirs; Name: "{localappdata}\Seam\updates"
Type: dirifempty; Name: "{app}"

[Code]
const
  RunKey = 'Software\Microsoft\Windows\CurrentVersion\Run';
  StillRunning = 'Seam is still running. Quit it from the tray icon, then try again';

{ True when Seam quit, or wasn't running. }
function QuitTray(const SettingsExe: String): Boolean;
var
  Code: Integer;
begin
  Result := Exec(SettingsExe, '--quit-tray', '', SW_HIDE, ewWaitUntilTerminated, Code) and (Code = 0);
end;

{ Closes any running Seam, a portable copy elsewhere included, so the one
  installed here is the one that runs afterwards. }
function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := '';
  ExtractTemporaryFile('seam-settings.exe');
  if not QuitTray(ExpandConstant('{tmp}\seam-settings.exe')) then
    Result := StillRunning;
end;

function InitializeUninstall(): Boolean;
var
  SettingsExe: String;
begin
  Result := True;
  SettingsExe := ExpandConstant('{app}\seam-settings.exe');
  if FileExists(SettingsExe) and not QuitTray(SettingsExe) then
  begin
    SuppressibleMsgBox(StillRunning, mbError, MB_OK, IDOK);
    Result := False;
  end;
end;

{ Whether a path, or a command that starts with one, is inside the install folder. }
function InsideApp(const Command: String): Boolean;
var
  Exe, Dir: String;
  Quote: Integer;
begin
  Exe := Trim(Command);
  if (Length(Exe) > 0) and (Exe[1] = '"') then
  begin
    Delete(Exe, 1, 1);
    Quote := Pos('"', Exe);
    if Quote > 0 then
      Exe := Copy(Exe, 1, Quote - 1);
  end;
  Dir := RemoveBackslash(ExpandConstant('{app}'));
  Result := Pos(Lowercase(Dir) + '\', Lowercase(Exe)) = 1;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Command: String;
  Marker: AnsiString;
  MarkerFile: String;
begin
  if CurUninstallStep <> usPostUninstall then
    Exit;

  { A portable copy may own autostart, so only a value that points here goes. }
  if RegQueryStringValue(HKCU, RunKey, 'Seam', Command) and InsideApp(Command) then
    RegDeleteValue(HKCU, RunKey, 'Seam');

  MarkerFile := ExpandConstant('{localappdata}\Seam\installed-unpackaged');
  if LoadStringFromFile(MarkerFile, Marker) and InsideApp(Utf8Decode(Marker)) then
    DeleteFile(MarkerFile);

  { Settings stay unless asked for. A silent uninstall keeps them. }
  if not UninstallSilent then
    if SuppressibleMsgBox('Also remove your settings? Your borders, zones and workspaces will be deleted',
      mbConfirmation, MB_YESNO or MB_DEFBUTTON2, IDNO) = IDYES then
      DelTree(ExpandConstant('{userappdata}\Seam'), True, True, True);
end;
