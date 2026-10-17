; FlowShot Inno Setup Script
#ifndef AppVersion
  #define AppVersion "0.4.0"
#endif

[Setup]
AppId={{8B84F093-5A59-4E1F-B19E-6E3E6DF61A45}
AppName=FlowShot
AppVersion={#AppVersion}
AppVerName=FlowShot {#AppVersion}
AppPublisher=FlowOSS
AppPublisherURL=https://github.com/flowoss/flowshot
AppSupportURL=https://github.com/flowoss/flowshot/issues
AppUpdatesURL=https://github.com/flowoss/flowshot/releases
DefaultDirName={autopf}\FlowShot
DefaultGroupName=FlowShot
AllowNoIcons=yes
LicenseFile=..\..\LICENSE
OutputDir=..\..\target\windows-dist
OutputBaseFilename=flowshot-setup-{#AppVersion}-x86_64
SetupIconFile=flowshot.ico
UninstallDisplayIcon={app}\flowshot.ico
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "startup"; Description: "Launch FlowShot when Windows starts"; GroupDescription: "Startup:"

[Files]
Source: "..\..\target\release\flowshot.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\target\release\flowshot-daemon.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "flowshot.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\FlowShot"; Filename: "{app}\flowshot-daemon.exe"; IconFilename: "{app}\flowshot.ico"
Name: "{group}\FlowShot (Capture)"; Filename: "{app}\flowshot.exe"; Parameters: "capture"; IconFilename: "{app}\flowshot.ico"
Name: "{group}\{cm:UninstallProgram,FlowShot}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\FlowShot"; Filename: "{app}\flowshot.exe"; Parameters: "capture"; Tasks: desktopicon; IconFilename: "{app}\flowshot.ico"

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "FlowShot"; ValueData: """{app}\flowshot-daemon.exe"""; Flags: uninsdeletevalue; Tasks: startup

[Run]
Filename: "{app}\flowshot-daemon.exe"; Description: "{cm:LaunchProgram,FlowShot}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "taskkill"; Parameters: "/f /im flowshot-daemon.exe"; Flags: runhidden; RunOnceId: "KillFlowShotDaemon"
