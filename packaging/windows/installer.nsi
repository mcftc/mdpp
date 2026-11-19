; MD++ Windows setup (NSIS 3). Built on Linux or Windows by scripts/release.sh:
;   makensis -DVERSION=0.1.0 -DSOURCE=<folder with mdpp.exe and the docs> -DOUTFILE=<setup.exe> installer.nsi
;
; Installs for the current user, without administrator rights (like VS Code's user setup):
; %LOCALAPPDATA%\Programs\MD++, Start menu entry, optional desktop shortcut, MD++ in "Open with"
; for Markdown files, `mdpp` runnable from Win+R, and an entry in Apps & features to uninstall.

Unicode true
!include "MUI2.nsh"
!include "FileFunc.nsh"

!ifndef VERSION
  !error "Pass -DVERSION=x.y.z"
!endif
!ifndef SOURCE
  !error "Pass -DSOURCE=<folder with mdpp.exe>"
!endif
!ifndef OUTFILE
  !define OUTFILE "MD++-${VERSION}-setup.exe"
!endif

!define APP_NAME "MD++"
!define APP_ID "io.github.mcftc.mdpp"
!define PROG_ID "MDPP.Markdown"
!define EXE "mdpp.exe"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_ID}"
!define APP_PATHS_KEY "Software\Microsoft\Windows\CurrentVersion\App Paths\${EXE}"

Name "${APP_NAME} ${VERSION}"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\${APP_NAME}"
InstallDirRegKey HKCU "${UNINSTALL_KEY}" "InstallLocation"
RequestExecutionLevel user
SetCompressor /SOLID lzma
ManifestDPIAware true

VIProductVersion "${VERSION}.0"
VIAddVersionKey /LANG=1033 "ProductName" "${APP_NAME}"
VIAddVersionKey /LANG=1033 "ProductVersion" "${VERSION}"
VIAddVersionKey /LANG=1033 "FileVersion" "${VERSION}"
VIAddVersionKey /LANG=1033 "FileDescription" "${APP_NAME} ${VERSION} setup"
VIAddVersionKey /LANG=1033 "LegalCopyright" "MIT OR Apache-2.0"

!define MUI_ICON "mdpp.ico"
!define MUI_UNICON "mdpp.ico"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\${EXE}"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${SOURCE}\LICENSE-MIT"
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

; The setup speaks the system's language when it is one of these.
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "Turkish"

LangString SecProgram ${LANG_ENGLISH} "MD++ (required)"
LangString SecProgram ${LANG_TURKISH} "MD++ (gerekli)"
LangString SecDesktop ${LANG_ENGLISH} "Desktop shortcut"
LangString SecDesktop ${LANG_TURKISH} "Masaüstü kısayolu"
LangString SecAssoc ${LANG_ENGLISH} "Open Markdown files (.md) with MD++"
LangString SecAssoc ${LANG_TURKISH} "Markdown dosyalarını (.md) MD++ ile aç"
LangString DescProgram ${LANG_ENGLISH} "The editor and its Start menu entry."
LangString DescProgram ${LANG_TURKISH} "Düzenleyici ve Başlat menüsü kaydı."
LangString DescDesktop ${LANG_ENGLISH} "An MD++ icon on the desktop."
LangString DescDesktop ${LANG_TURKISH} "Masaüstünde bir MD++ simgesi."
LangString DescAssoc ${LANG_ENGLISH} "Adds MD++ to Open with for .md and .markdown files (Windows asks before changing the default)."
LangString DescAssoc ${LANG_TURKISH} "MD++'yı .md ve .markdown dosyaları için Birlikte aç listesine ekler (varsayılanı değiştirmeden önce Windows sorar)."
LangString FileTypeName ${LANG_ENGLISH} "Markdown document"
LangString FileTypeName ${LANG_TURKISH} "Markdown belgesi"
LangString StillRunning ${LANG_ENGLISH} "MD++ is running. Close it and press Retry."
LangString StillRunning ${LANG_TURKISH} "MD++ açık. Kapatıp Yeniden Dene'ye bas."

; A running MD++ keeps mdpp.exe locked: ask the user to close it.
!macro CheckNotRunning
  retry:
  ClearErrors
  IfFileExists "$INSTDIR\${EXE}" 0 done
  FileOpen $0 "$INSTDIR\${EXE}" a
  IfErrors 0 closed
  MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "$(StillRunning)" IDRETRY retry
  Abort
  closed:
  FileClose $0
  done:
!macroend

Section "$(SecProgram)" SecProgram
  SectionIn RO
  SetOutPath "$INSTDIR"
  !insertmacro CheckNotRunning
  File "${SOURCE}\${EXE}"
  File "${SOURCE}\README.md"
  File "${SOURCE}\LICENSE-MIT"
  File "${SOURCE}\LICENSE-APACHE"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  CreateShortcut "$SMPROGRAMS\${APP_NAME}.lnk" "$INSTDIR\${EXE}"

  ; Win+R → mdpp
  WriteRegStr HKCU "${APP_PATHS_KEY}" "" "$INSTDIR\${EXE}"
  WriteRegStr HKCU "${APP_PATHS_KEY}" "Path" "$INSTDIR"

  ; Apps & features
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "${APP_NAME}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "MD++ contributors"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "URLInfoAbout" "https://github.com/mcftc/mdpp"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\${EXE}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINSTALL_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "EstimatedSize" "$0"
SectionEnd

Section "$(SecDesktop)" SecDesktop
  CreateShortcut "$DESKTOP\${APP_NAME}.lnk" "$INSTDIR\${EXE}"
SectionEnd

Section "$(SecAssoc)" SecAssoc
  WriteRegStr HKCU "Software\Classes\${PROG_ID}" "" "$(FileTypeName)"
  WriteRegStr HKCU "Software\Classes\${PROG_ID}\DefaultIcon" "" "$INSTDIR\${EXE},0"
  WriteRegStr HKCU "Software\Classes\${PROG_ID}\shell\open\command" "" '"$INSTDIR\${EXE}" "%1"'
  WriteRegStr HKCU "Software\Classes\.md\OpenWithProgids" "${PROG_ID}" ""
  WriteRegStr HKCU "Software\Classes\.markdown\OpenWithProgids" "${PROG_ID}" ""
  WriteRegStr HKCU "Software\Classes\Applications\${EXE}\shell\open\command" "" '"$INSTDIR\${EXE}" "%1"'
  WriteRegStr HKCU "Software\Classes\Applications\${EXE}\SupportedTypes" ".md" ""
  WriteRegStr HKCU "Software\Classes\Applications\${EXE}\SupportedTypes" ".markdown" ""
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
SectionEnd

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecProgram} "$(DescProgram)"
  !insertmacro MUI_DESCRIPTION_TEXT ${SecDesktop} "$(DescDesktop)"
  !insertmacro MUI_DESCRIPTION_TEXT ${SecAssoc} "$(DescAssoc)"
!insertmacro MUI_FUNCTION_DESCRIPTION_END

Section "Uninstall"
  !insertmacro CheckNotRunning
  Delete "$INSTDIR\${EXE}"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\LICENSE-MIT"
  Delete "$INSTDIR\LICENSE-APACHE"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${APP_NAME}.lnk"
  Delete "$DESKTOP\${APP_NAME}.lnk"
  DeleteRegKey HKCU "${UNINSTALL_KEY}"
  DeleteRegKey HKCU "${APP_PATHS_KEY}"
  DeleteRegKey HKCU "Software\Classes\${PROG_ID}"
  DeleteRegValue HKCU "Software\Classes\.md\OpenWithProgids" "${PROG_ID}"
  DeleteRegValue HKCU "Software\Classes\.markdown\OpenWithProgids" "${PROG_ID}"
  DeleteRegKey HKCU "Software\Classes\Applications\${EXE}"
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
  ; Settings, session and backups stay in %APPDATA%\mdpp — the user's data.
SectionEnd
