; Установщик TVOICE (Inno Setup 6).
;
; Установка ПОЛЬЗОВАТЕЛЬСКАЯ, в %LOCALAPPDATA%\Programs\TVOICE, без прав администратора.
; Это не стилистический выбор: программа хранит настройки, журнал, движок whisper.cpp и
; модели РЯДОМ со своим .exe (см. models::app_dir). В Program Files обычный пользователь
; писать не может, поэтому там сломались бы и сохранение настроек, и загрузка моделей.
;
; Сборка: iscc installer\tvoice.iss /DAppVersion=1.21.1
; Проще — scripts\build-installer.ps1: он сам возьмёт версию из Cargo.toml.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

#define AppName "TVOICE"
#define AppPublisher "DjonBeyron"
#define AppURL "https://github.com/DjonBeyron/Tvoice"
#define AppExe "tvoice.exe"

[Setup]
; GUID постоянный: по нему Windows опознаёт установку при обновлении. Менять нельзя —
; иначе новая версия встанет рядом со старой, а не поверх.
AppId={{7A3F1D62-9C48-4E5B-A1F7-2D8B6C0E4931}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
AppPublisherURL={#AppURL}
AppSupportURL={#AppURL}/issues
AppUpdatesURL={#AppURL}/releases
VersionInfoVersion={#AppVersion}

; Пользовательская установка: без UAC и без записи в общие папки.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
DefaultDirName={localappdata}\Programs\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
DisableDirPage=auto
AllowNoIcons=yes

OutputDir=..\dist
OutputBaseFilename=TVOICE-{#AppVersion}-setup
SetupIconFile=..\assets\tvoice.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName} {#AppVersion}

Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
; Именно x64compatible, а не x64: последнее Inno подменяет на x64os, то есть «только родная
; x64-система», и установка на Windows на ARM оказалась бы запрещена — хотя x64-программы
; там работают через эмуляцию. Требует Inno Setup 6.3+.
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

; Программа рассчитана на Windows 10 1809+ / Windows 11 (WinRT, WASAPI, Media Foundation).
MinVersion=10.0.17763

[Languages]
Name: "ru"; MessagesFile: "compiler:Languages\Russian.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "autostart"; Description: "{cm:AutoStart}"; GroupDescription: "{cm:AdditionalOptions}"

[Files]
Source: "..\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
; Сигналы входа в диктовку и выхода из неё. Обратный сигнал программа соберёт из этого же
; файла при первом запуске, поэтому второй звук в комплект не нужен.
Source: "..\assets\rec.mp3"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.ru.md"; DestDir: "{app}"; Flags: ignoreversion isreadme
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{group}\{cm:UninstallProgram,{#AppName}}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Registry]
; Автозапуск — то же значение, которым управляет галочка в настройках программы
; (см. src/autostart.rs). Флаг --tray нужен, чтобы при старте системы окно не выпрыгивало.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; \
    ValueName: "TVOICE"; ValueData: """{app}\{#AppExe}"" --tray"; \
    Flags: uninsdeletevalue; Tasks: autostart

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; \
    Flags: nowait postinstall skipifsilent

[UninstallDelete]
; Производные файлы, которые программа создаёт сама. Настройки, скачанные модели и движок
; НЕ удаляем: модели весят сотни мегабайт, и повторно качать их после переустановки никто
; не поблагодарит. Полную очистку пользователь делает удалением папки вручную.
;
; Журнал удаляем: это диагностика, а не данные. Иначе он один оставался бы в папке, и та
; не удалялась бы вовсе — даже когда моделей нет и удалять больше нечего.
Type: filesandordirs; Name: "{app}\temp"
Type: files; Name: "{app}\tvoice.log"
Type: files; Name: "{app}\debug\empty_*.wav"
Type: dirifempty; Name: "{app}\debug"

[CustomMessages]
ru.AutoStart=Запускать TVOICE при входе в Windows (свёрнутым в трей)
en.AutoStart=Start TVOICE when you sign in to Windows (minimised to tray)
ru.AdditionalOptions=Дополнительно:
en.AdditionalOptions=Additional options:

[Code]
// Не даём ставить поверх запущенной копии: файл был бы занят, и установка упала бы
// на середине. Программа держит единственный экземпляр через именованный мьютекс
// (src/single.rs) — по нему и проверяем.
// Закрыть запущенную копию. Возвращает True, если путь свободен.
function CloseRunning(): Boolean;
var
  ResultCode: Integer;
begin
  Exec('taskkill.exe', '/F /IM tvoice.exe', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Sleep(1500);
  Result := not CheckForMutexes('TVOICE_single_instance_v1');
end;

function InitializeSetup(): Boolean;
begin
  Result := True;
  if not CheckForMutexes('TVOICE_single_instance_v1') then
    Exit;
  if MsgBox('TVOICE запущен. Закрыть его и продолжить установку?' + #13#10 +
            'TVOICE is running. Close it and continue?',
            mbConfirmation, MB_YESNO) = IDYES then
    Result := CloseRunning()
  else
    Result := False;
end;

function InitializeUninstall(): Boolean;
begin
  Result := True;
  if CheckForMutexes('TVOICE_single_instance_v1') then
    Result := CloseRunning();
end;
