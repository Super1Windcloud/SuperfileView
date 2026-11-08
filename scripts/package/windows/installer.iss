; Inno Setup Script for Superfile
; Cross-platform high-performance desktop file viewer and editor

#ifndef AppVersion
#define AppVersion "0.0.1"
#endif

#ifndef SourceExe
#define SourceExe "..\..\..\target\x86_64-pc-windows-msvc\release\superfile.exe"
#endif

#ifndef OutputDir
#define OutputDir "..\..\..\package-dist"
#endif

#ifndef OutputFileName
#define OutputFileName "superfile-windows-x86_64-setup"
#endif

[Setup]
AppId={{9F8F8256-42E2-4C5D-876B-C877DDF7106D}
AppName=Superfile
AppVersion={#AppVersion}
AppPublisher=Superfile Team
AppPublisherURL=https://github.com/superfile
AppSupportURL=https://github.com/superfile
AppUpdatesURL=https://github.com/superfile
DefaultDirName={autopf}\Superfile
DisableProgramGroupPage=yes
OutputDir={#OutputDir}
OutputBaseFilename={#OutputFileName}
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
UninstallDisplayIcon={app}\superfile.exe

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "chinesesimplified"; MessagesFile: "compiler:Languages\ChineseSimplified.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\..\README.md"; DestDir: "{app}"; Flags: ignoreversion; Flags: skipifsourcedoesntexist
Source: "..\..\..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion; Flags: skipifsourcedoesntexist

[Icons]
Name: "{autoprograms}\Superfile"; Filename: "{app}\superfile.exe"
Name: "{autodesktop}\Superfile"; Filename: "{app}\superfile.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\superfile.exe"; Description: "{cm:LaunchProgram,Superfile}"; Flags: nowait postinstall skipifsilent
