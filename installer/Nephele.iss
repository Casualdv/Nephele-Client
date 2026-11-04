#define NepheleVersion GetEnv("NEPHELE_APP_VERSION")
#define StageDir GetEnv("NEPHELE_STAGE_DIR")

[Setup]
AppId={{2B4265F2-A463-4CF0-9DA1-2645B3C8566E}
AppName=Nephele
AppVersion={#NepheleVersion}
AppPublisher=Nephele Project
AppPublisherURL=https://github.com/Casualdv/Nephele-Client
AppSupportURL=https://github.com/Casualdv/Nephele-Client/issues
AppUpdatesURL=https://github.com/Casualdv/Nephele-Client/releases
DefaultDirName={localappdata}\Programs\Nephele
DefaultGroupName=Nephele
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64
ArchitecturesInstallIn64BitMode=x64
SetupIconFile={#StageDir}\nephele.ico
UninstallDisplayIcon={app}\nephele.ico
OutputDir=..\dist
OutputBaseFilename=NepheleSetup-{#NepheleVersion}-win-x64
WizardStyle=modern
Compression=lzma2/ultra64
SolidCompression=yes
CloseApplications=yes
CloseApplicationsFilter=nephele.exe;nephele-launcher.exe;nephele-prepare.exe
Uninstallable=yes
ChangesAssociations=no

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: unchecked

[Files]
Source: "{#StageDir}\.nephele-install"; DestDir: "{app}"; Flags: ignoreversion hidden
Source: "{#StageDir}\current-version.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\nephele.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\versions\{#NepheleVersion}\nephele-launcher.exe"; DestDir: "{app}\versions\{#NepheleVersion}"; Flags: ignoreversion
Source: "{#StageDir}\versions\{#NepheleVersion}\nephele.exe"; DestDir: "{app}\versions\{#NepheleVersion}"; Flags: ignoreversion
Source: "{#StageDir}\versions\{#NepheleVersion}\nephele-prepare.exe"; DestDir: "{app}\versions\{#NepheleVersion}"; Flags: ignoreversion
Source: "{#StageDir}\versions\{#NepheleVersion}\research-lock.json"; DestDir: "{app}\versions\{#NepheleVersion}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Nephele"; Filename: "{app}\versions\{#NepheleVersion}\nephele-launcher.exe"; Parameters: "--install-root ""{app}"""; IconFilename: "{app}\nephele.ico"
Name: "{autodesktop}\Nephele"; Filename: "{app}\versions\{#NepheleVersion}\nephele-launcher.exe"; Parameters: "--install-root ""{app}"""; IconFilename: "{app}\nephele.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\versions\{#NepheleVersion}\nephele-launcher.exe"; Parameters: "--install-root ""{app}"""; Description: "Launch Nephele"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
Type: filesandordirs; Name: "{app}\versions"
Type: files; Name: "{app}\current-version.txt"
Type: files; Name: "{app}\.nephele-install"
Type: files; Name: "{app}\nephele.ico"
