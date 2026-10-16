; Instalador do Ampero para Windows (Inno Setup 6).
; Gere com installer\build.ps1, que passa a versão do Cargo.toml e a pasta dos executáveis.

#define AppName "Ampero"
#define AppExe "ampero-gui.exe"
#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef BinDir
  #define BinDir "..\target\release"
#endif

[Setup]
; O app procura esta chave no registro para saber se pode se atualizar sozinho (self_update.rs).
AppId={{50676992-6BAD-419D-914B-010B3C748C3B}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=macedo
AppPublisherURL=https://github.com/macedo/ampero-re
AppSupportURL=https://github.com/macedo/ampero-re/issues
AppUpdatesURL=https://github.com/macedo/ampero-re/releases
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
; Instalação por usuário: sem pedir administrador, e a atualização pelo app também não pede.
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir=..\dist
OutputBaseFilename=ampero-{#AppVersion}-windows-x64-setup
SetupIconFile=..\assets\icon.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
; Fecha o app aberto antes de trocar os arquivos (usado na atualização pelo próprio app).
CloseApplications=yes

[Languages]
Name: "brazilianportuguese"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "{#BinDir}\ampero-gui.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BinDir}\ampero.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent
; Atualização pelo app: ele roda este instalador em silêncio com /RESTARTAPP=1 e fecha;
; a versão nova abre assim que terminar.
Filename: "{app}\{#AppExe}"; Flags: nowait; Check: RestartRequested

; As configurações do usuário (%APPDATA%\ampero: tema, assistente) ficam ao desinstalar, de propósito:
; reinstalar não perde a chave da API nem o prompt personalizado.

[Code]
function RestartRequested(): Boolean;
begin
  Result := ExpandConstant('{param:RESTARTAPP|0}') = '1';
end;
