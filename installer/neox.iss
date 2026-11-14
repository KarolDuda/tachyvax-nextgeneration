#define MyAppName "NEOX"
#ifndef MyAppVersion
  #define MyAppVersion "3.0.0"
#endif
#define MyAppExe "Neox.exe"

[Setup]
; Fixed AppId: new versions upgrade the installed one instead of installing side by side.
AppId={{7C2E9B14-3A58-4F0D-8B6E-1D95A0C4E7F3}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher=spinprint
AppPublisherURL=https://spinprint.pl
AppSupportURL=https://spinprint.pl
AppCopyright=© spinprint (spinprint.pl)
VersionInfoCompany=spinprint
VersionInfoProductName=NEOX
VersionInfoProductVersion={#MyAppVersion}
VersionInfoVersion={#MyAppVersion}.0
VersionInfoDescription=NEOX Setup
VersionInfoCopyright=© spinprint (spinprint.pl)
VersionInfoOriginalFileName=Neox_Setup.exe
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
; Installing to Program Files needs admin; the app itself starts without admin (asInvoker).
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=output
OutputBaseFilename=Neox_Setup
; Stored without compression on purpose: a small, plain payload looks less like a packer to ML scanners than a high-entropy LZMA blob.
Compression=none
SolidCompression=no
SetupIconFile=..\app\icons\icon.ico
WizardStyle=modern
UninstallDisplayIcon={app}\{#MyAppExe}
CloseApplications=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "..\publish\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\{#MyAppName}"; Filename: "{app}\{#MyAppExe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExe}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExe}"; Description: "Launch {#MyAppName}"; Flags: nowait postinstall skipifsilent runasoriginaluser
; Background update (silent mode): relaunch the app after install, without admin rights.
Filename: "{app}\{#MyAppExe}"; Flags: nowait runasoriginaluser; Check: WizardSilent
