; Setup für SpedHub, Inno Setup 6. Ergebnis ist eine einzelne SpedHub-Setup-<Version>.exe.
;
; Gebaut wird es mit installer\build.ps1, das vorher alles im Release-Modus baut und dann
;   ISCC.exe /DAppVersion=0.2.0 /DRootDir=C:\dev\SpedHub /DOutputDir=C:\dev\SpedHub\dist installer\spedhub.iss
; aufruft. Ohne /D-Angaben gilt die Version 0.0.0, die Dateien kommen aus ..\target\release.
;
; Die Datei ist UTF-8 mit BOM gespeichert, sonst liest Inno Setup die Umlaute falsch.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef RootDir
  #define RootDir AddBackslash(SourcePath) + ".."
#endif
#ifndef OutputDir
  #define OutputDir RootDir + "\dist"
#endif
#define BinDir RootDir + "\target\release"
#define AppExe "spedhub-launcher.exe"
; Hier meldet der Launcher mit Adminrechten einen von Hand gewählten Spielordner
#define GameDirReport "spielordner.txt"

[Setup]
; Feste Kennung, damit ein neues Setup die alte Installation ersetzt. Nie ändern.
AppId={{6E2B9C47-3F1A-4D8E-A5B2-91C07D4E8F36}
AppName=SpedHub
AppVersion={#AppVersion}
AppVerName=SpedHub {#AppVersion}
AppPublisher=SpedHub
AppPublisherURL=https://github.com/LSSippler/SpedHub
AppSupportURL=https://github.com/LSSippler/SpedHub/issues
AppUpdatesURL=https://github.com/LSSippler/SpedHub/releases
DefaultDirName={autopf}\SpedHub
DefaultGroupName=SpedHub
DisableProgramGroupPage=yes
; Adminrechte: Program Files, und das Plugin kommt in den Spielordner
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutputDir}
OutputBaseFilename=SpedHub-Setup-{#AppVersion}
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName=SpedHub
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ShowLanguageDialog=no
; Ein laufender Launcher wird für das Update geschlossen
CloseApplications=yes
RestartApplications=no
VersionInfoProductName=SpedHub
VersionInfoProductTextVersion={#AppVersion}
VersionInfoDescription=SpedHub Setup

[Languages]
Name: "german"; MessagesFile: "compiler:Languages\German.isl"

[Types]
Name: "standard"; Description: "Launcher und Plugin"
Name: "custom"; Description: "Benutzerdefiniert"; Flags: iscustom

[Components]
Name: "main"; Description: "SpedHub-Launcher und Plugin für Euro Truck Simulator 2"; Types: standard custom; Flags: fixed
; In keinem Typ enthalten, also standardmäßig abgewählt
Name: "server"; Description: "Server für lokale Tests"

[Tasks]
Name: "desktopicon"; Description: "Verknüpfung auf dem Desktop"; GroupDescription: "Verknüpfungen:"

[Files]
Source: "{#BinDir}\spedhub-launcher.exe"; DestDir: "{app}"; Components: main; Flags: ignoreversion
; Die DLL liegt neben dem Launcher, von dort kopiert --install-plugin sie ins Spiel
Source: "{#BinDir}\spedhub_plugin.dll"; DestDir: "{app}"; Components: main; Flags: ignoreversion
; Kommandozeile für Spielstände und zum Bauen der Wirtschafts-Mod, siehe Wirtschafts-Mod.md
Source: "{#BinDir}\spedhub-save.exe"; DestDir: "{app}"; Components: main; Flags: ignoreversion
Source: "{#RootDir}\docs\wirtschafts-mod.md"; DestDir: "{app}"; DestName: "Wirtschafts-Mod.md"; Components: main; Flags: ignoreversion
Source: "{#RootDir}\README.md"; DestDir: "{app}"; Components: main; Flags: ignoreversion
Source: "{#RootDir}\crates\spedhub-launcher\README.md"; DestDir: "{app}"; DestName: "README-Launcher.md"; Components: main; Flags: ignoreversion
Source: "{#RootDir}\crates\spedhub-plugin\README.md"; DestDir: "{app}"; DestName: "README-Plugin.md"; Components: main; Flags: ignoreversion
#if FileExists(RootDir + "\LICENSE")
Source: "{#RootDir}\LICENSE"; DestDir: "{app}"; Components: main; Flags: ignoreversion
#endif
Source: "{#BinDir}\spedhub-server.exe"; DestDir: "{app}"; Components: server; Flags: ignoreversion
Source: "{#RootDir}\server\README.md"; DestDir: "{app}"; DestName: "README-Server.md"; Components: server; Flags: ignoreversion

[Icons]
Name: "{group}\SpedHub"; Filename: "{app}\{#AppExe}"; Components: main
Name: "{autodesktop}\SpedHub"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon
; --local: Datenbank in %LOCALAPPDATA%\SpedHub\server des Benutzers, der die Verknüpfung startet
Name: "{group}\SpedHub-Server (lokaler Test)"; Filename: "{app}\spedhub-server.exe"; Parameters: "--local"; WorkingDir: "{app}"; Components: server
Name: "{group}\SpedHub deinstallieren"; Filename: "{uninstallexe}"

[Run]
; Läuft mit den Adminrechten des Setups. Findet der Launcher das Spiel nicht, sagt er das und
; lässt den Ordner wählen. Bei einer stillen Installation ohne Fenster.
;
; Hat ein Standardbenutzer im UAC-Dialog die Anmeldung eines Admins eingegeben, läuft das Setup
; mit dessen Konto, ein hier gewählter Spielordner landete in dessen launcher.json. Deshalb
; schreibt der Launcher einen von Hand gewählten Ordner in {tmp}\spielordner.txt, und die
; dritte Zeile merkt ihn als der Benutzer, der das Setup gestartet hat.
Filename: "{app}\{#AppExe}"; Parameters: "--install-plugin --report-game-dir ""{tmp}\{#GameDirReport}"""; StatusMsg: "Plugin für Euro Truck Simulator 2 wird installiert"; Flags: waituntilterminated; Check: not WizardSilent
Filename: "{app}\{#AppExe}"; Parameters: "--install-plugin --quiet"; StatusMsg: "Plugin für Euro Truck Simulator 2 wird installiert"; Flags: waituntilterminated; Check: WizardSilent
Filename: "{app}\{#AppExe}"; Parameters: "--remember-game-dir {code:ChosenGameDir}"; StatusMsg: "Spielordner wird gemerkt"; Flags: waituntilterminated runasoriginaluser; Check: GameDirChosen
Filename: "{app}\{#AppExe}"; Description: "SpedHub starten"; Flags: postinstall nowait skipifsilent runasoriginaluser

; Das Plugin entfernt [Code] (RemovePlugin), nicht [UninstallRun]: Dessen Check wird schon bei
; der Installation ausgewertet, eine stille Deinstallation ließe sich dort nicht erkennen.

[Code]
var
  KeepUserData: Boolean;

{ Der Spielordner, den der Launcher mit Adminrechten von Hand wählen ließ, leer, wenn keiner.
  Die Datei ist UTF-8 mit BOM. }
function ReportedGameDir: String;
var
  Lines: TArrayOfString;
begin
  Result := '';
  if LoadStringsFromFile(ExpandConstant('{tmp}\{#GameDirReport}'), Lines) then
    if GetArrayLength(Lines) > 0 then
      Result := Trim(Lines[0]);
end;

function GameDirChosen: Boolean;
begin
  Result := ReportedGameDir <> '';
end;

{ Ohne Backslash am Ende, der würde sonst das schließende Anführungszeichen schützen }
function ChosenGameDir(Param: String): String;
begin
  Result := AddQuotes(RemoveBackslashUnlessRoot(ReportedGameDir));
end;

{ Entfernt das Plugin aus dem Spiel, solange der Launcher noch da ist. Läuft das Spiel, fragt
  der Launcher mit Wiederholen und Abbrechen. Bei Abbrechen und bei einer stillen
  Deinstallation (--quiet, ohne Fenster) benennt er die DLL um, damit das Spiel sie nicht mehr
  lädt, und merkt sie zum Löschen beim nächsten Neustart von Windows vor. Die Deinstallation
  geht in jedem Fall weiter. }
procedure RemovePlugin;
var
  Params: String;
  ResultCode: Integer;
begin
  Params := '--uninstall-plugin';
  if UninstallSilent then
    Params := Params + ' --quiet';
  if not Exec(ExpandConstant('{app}\{#AppExe}'), Params, '', SW_SHOWNORMAL, ewWaitUntilTerminated, ResultCode) then
    Log('Plugin entfernen: Der Launcher ließ sich nicht starten, Fehler ' + IntToStr(ResultCode))
  else if ResultCode <> 0 then
    Log('Plugin entfernen: Der Launcher endete mit Code ' + IntToStr(ResultCode));
end;

{ Anmeldung, Einstellungen, Backups, Baupläne und die Datenbank des lokalen Servers liegen in
  %LOCALAPPDATA%\SpedHub. Gemeint ist der Benutzer, der deinstalliert: Läuft die
  Deinstallation mit einem anderen Admin-Konto, ist es dessen Ordner. Standard ist behalten,
  bei einer stillen Deinstallation wird nie gelöscht. }
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  DataDir: String;
begin
  DataDir := ExpandConstant('{localappdata}\SpedHub');
  if CurUninstallStep = usUninstall then
  begin
    { Zuerst, er liest den gemerkten Spielordner aus %LOCALAPPDATA%\SpedHub }
    RemovePlugin;
    KeepUserData := True;
    if (not UninstallSilent) and DirExists(DataDir) then
      KeepUserData := MsgBox('Anmeldung, Einstellungen und Backups behalten?' + #13#10#13#10 +
        'Sie liegen in ' + DataDir + '. Mit Nein werden sie gelöscht, auch die Backups deiner Spielstände.',
        mbConfirmation, MB_YESNO or MB_DEFBUTTON1) = IDYES;
  end;
  if (CurUninstallStep = usPostUninstall) and (not KeepUserData) then
  begin
    if DirExists(DataDir) then
      DelTree(DataDir, True, True, True);
  end;
end;
