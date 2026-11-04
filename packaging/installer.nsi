; SentinelTask Windows installer (NSIS 3, Modern UI 2).
;
; Build:  makensis -DVERSION=1.1.0 -DSRCDIR=<folder with SentinelTask.exe> -DOUTFILE=<setup.exe> installer.nsi
;
; Design choices:
; * Per-user install (%LOCALAPPDATA%\Programs\SentinelTask) — no UAC prompt,
;   same as VS Code / Discord user installers. SentinelTask itself offers
;   "Restart as administrator" when a target program needs it.
; * The uninstaller removes only the files it installed, never the folder
;   recursively, so choosing an odd install folder can't delete user data.
;   Macros and settings are kept.
; * Detects a running SentinelTask (named mutex) before replacing files.
; * Auto-update: SentinelTask runs this installer as `setup.exe /S /UPDATE`
;   (plus `/RELAUNCH` to start the new version afterwards). Update mode
;   waits for SentinelTask to close instead of asking, and keeps the
;   shortcut and file-association choices of the existing install.

; SentinelTask is x64-only, so the installer is a native 64-bit program too
; (no WOW64 registry/file redirection surprises).
Target amd64-unicode
ManifestDPIAware true
SetCompressor /SOLID lzma

!ifndef VERSION
  !define VERSION "1.1.0"
!endif
!ifndef SRCDIR
  !define SRCDIR "..\target\release"
!endif
!ifndef OUTFILE
  !define OUTFILE "SentinelTask-${VERSION}-setup.exe"
!endif
!ifndef DOCDIR
  !define DOCDIR ".."
!endif

!define APP       "SentinelTask"
!define EXE       "SentinelTask.exe"
!define PUBLISHER "SentinelTask contributors"
!define URL       "https://github.com/DestroyersTaken/SentinelTask"
!define UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP}"
!define REGKEY    "Software\${APP}"
!define PROGID    "SentinelTask.Macro"

Name "${APP} ${VERSION}"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\${APP}"
InstallDirRegKey HKCU "${REGKEY}" "InstallDir"
RequestExecutionLevel user
BrandingText "${APP} ${VERSION}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${APP}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "FileDescription" "${APP} installer"
VIAddVersionKey "CompanyName" "${PUBLISHER}"
VIAddVersionKey "LegalCopyright" "MIT License"

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"
!include "Sections.nsh"

!define MUI_ICON   "..\crates\sentineltask\assets\sentineltask.ico"
!define MUI_UNICON "..\crates\sentineltask\assets\sentineltask.ico"
!define MUI_WELCOMEFINISHPAGE_BITMAP   "wizard.bmp"
!define MUI_UNWELCOMEFINISHPAGE_BITMAP "wizard.bmp"
!define MUI_HEADERIMAGE
!define MUI_HEADERIMAGE_RIGHT
!define MUI_HEADERIMAGE_BITMAP "header.bmp"
!define MUI_ABORTWARNING
!define MUI_COMPONENTSPAGE_SMALLDESC

!define MUI_WELCOMEPAGE_TITLE "Welcome to ${APP} ${VERSION}"
!define MUI_WELCOMEPAGE_TEXT "${APP} records your mouse and keyboard and replays them exactly — a modern, reliable successor to TinyTask.$\r$\n$\r$\nNo administrator rights are needed to install.$\r$\n$\r$\nClick Next to continue."
!define MUI_FINISHPAGE_RUN "$INSTDIR\${EXE}"
!define MUI_FINISHPAGE_RUN_TEXT "Start ${APP} now"
!define MUI_FINISHPAGE_LINK "Project page and help"
!define MUI_FINISHPAGE_LINK_LOCATION "${URL}"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${DOCDIR}\LICENSE"
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_UNPAGE_FINISH

!insertmacro MUI_LANGUAGE "English"

; ---------------------------------------------------------------------------

!macro EnsureNotRunning UN
Function ${UN}EnsureNotRunning
  retry:
    System::Call 'kernel32::OpenMutexW(i 0x00100000, i 0, w "SentinelTask.Running") p .r0'
    ${If} $0 P<> 0
      System::Call 'kernel32::CloseHandle(p r0)'
      MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION \
        "${APP} is running.$\r$\n$\r$\nPlease close it (stop any playback first), then click Retry." \
        /SD IDCANCEL IDRETRY retry
      Abort
    ${EndIf}
FunctionEnd
!macroend
!insertmacro EnsureNotRunning ""
!insertmacro EnsureNotRunning "un."

Var UpdateMode
Var Relaunch

; Update mode: SentinelTask has just been asked to close; give it up to a
; minute (it finishes saving and releases held keys first).
Function WaitUntilClosed
  StrCpy $1 0
  wait:
    System::Call 'kernel32::OpenMutexW(i 0x00100000, i 0, w "SentinelTask.Running") p .r0'
    ${If} $0 P<> 0
      System::Call 'kernel32::CloseHandle(p r0)'
      IntOp $1 $1 + 1
      ${If} $1 > 240
        Abort
      ${EndIf}
      Sleep 250
      Goto wait
    ${EndIf}
  ; The program file is unlocked a moment after the process ends.
  Sleep 500
FunctionEnd

Function un.onInit
  Call un.EnsureNotRunning
FunctionEnd

; ---------------------------------------------------------------------------

Section "${APP} (required)" SecApp
  SectionIn RO
  SetOutPath "$INSTDIR"
  File "${SRCDIR}\${EXE}"
  File "/oname=README.md" "${DOCDIR}\README.md"
  File "/oname=LICENSE.txt" "${DOCDIR}\LICENSE"
  WriteUninstaller "$INSTDIR\Uninstall ${APP}.exe"

  CreateShortCut "$SMPROGRAMS\${APP}.lnk" "$INSTDIR\${EXE}" "" "$INSTDIR\${EXE}" 0 \
    SW_SHOWNORMAL "" "Record and replay mouse and keyboard macros"

  WriteRegStr HKCU "${REGKEY}" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayName" "${APP}"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTKEY}" "Publisher" "${PUBLISHER}"
  WriteRegStr HKCU "${UNINSTKEY}" "URLInfoAbout" "${URL}"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayIcon" "$INSTDIR\${EXE},0"
  WriteRegStr HKCU "${UNINSTKEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTKEY}" "UninstallString" '"$INSTDIR\Uninstall ${APP}.exe"'
  WriteRegStr HKCU "${UNINSTKEY}" "QuietUninstallString" '"$INSTDIR\Uninstall ${APP}.exe" /S'
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD HKCU "${UNINSTKEY}" "EstimatedSize" "$0"
SectionEnd

Section "Desktop shortcut" SecDesktop
  CreateShortCut "$DESKTOP\${APP}.lnk" "$INSTDIR\${EXE}" "" "$INSTDIR\${EXE}" 0
SectionEnd

Section "Open .stmacro files with ${APP}" SecAssoc
  WriteRegStr HKCU "Software\Classes\.stmacro" "" "${PROGID}"
  WriteRegStr HKCU "Software\Classes\${PROGID}" "" "SentinelTask macro"
  WriteRegStr HKCU "Software\Classes\${PROGID}\DefaultIcon" "" "$INSTDIR\${EXE},0"
  WriteRegStr HKCU "Software\Classes\${PROGID}\shell\open\command" "" '"$INSTDIR\${EXE}" "%1"'
  WriteRegStr HKCU "Software\Classes\${PROGID}\shell\play" "" "Play with ${APP}"
  WriteRegStr HKCU "Software\Classes\${PROGID}\shell\play\command" "" '"$INSTDIR\${EXE}" play "%1"'
  WriteRegStr HKCU "${REGKEY}" "AssocStmacro" "1"
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
SectionEnd

Section /o "Open TinyTask .rec files with ${APP}" SecRec
  WriteRegStr HKCU "Software\Classes\.rec\OpenWithProgids" "${PROGID}.Rec" ""
  WriteRegStr HKCU "Software\Classes\${PROGID}.Rec" "" "TinyTask recording"
  WriteRegStr HKCU "Software\Classes\${PROGID}.Rec\DefaultIcon" "" "$INSTDIR\${EXE},0"
  WriteRegStr HKCU "Software\Classes\${PROGID}.Rec\shell\open\command" "" '"$INSTDIR\${EXE}" "%1"'
  WriteRegStr HKCU "${REGKEY}" "AssocRec" "1"
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
SectionEnd

; .onInit comes after the sections so it can refer to their ids.
Function .onInit
  ${GetParameters} $R0
  ClearErrors
  ${GetOptions} $R0 "/UPDATE" $R1
  ${IfNot} ${Errors}
    StrCpy $UpdateMode 1
  ${EndIf}
  ClearErrors
  ${GetOptions} $R0 "/RELAUNCH" $R1
  ${IfNot} ${Errors}
    StrCpy $Relaunch 1
  ${EndIf}

  ${If} $UpdateMode == 1
    Call WaitUntilClosed
    ; Keep what the user chose when they first installed.
    ${IfNot} ${FileExists} "$DESKTOP\${APP}.lnk"
      SectionSetFlags ${SecDesktop} 0
    ${EndIf}
    ReadRegStr $0 HKCU "${REGKEY}" "AssocStmacro"
    ${If} $0 != "1"
      SectionSetFlags ${SecAssoc} 0
    ${EndIf}
    ReadRegStr $0 HKCU "${REGKEY}" "AssocRec"
    ${If} $0 == "1"
      SectionSetFlags ${SecRec} ${SF_SELECTED}
    ${EndIf}
  ${Else}
    Call EnsureNotRunning
  ${EndIf}
FunctionEnd

Function .onInstSuccess
  ${If} $Relaunch == 1
    Exec '"$INSTDIR\${EXE}"'
  ${EndIf}
FunctionEnd

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecApp} "The ${APP} program and a Start menu shortcut."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecDesktop} "Put a ${APP} shortcut on the desktop."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecAssoc} "Double-click a .stmacro file to open it; right-click → Play to run it."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecRec} "Offer ${APP} in “Open with” for TinyTask .rec recordings."
!insertmacro MUI_FUNCTION_DESCRIPTION_END

; ---------------------------------------------------------------------------

Section "Uninstall"
  ; Only the files we installed — never a recursive delete of the folder.
  Delete "$INSTDIR\${EXE}"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\LICENSE.txt"
  Delete "$INSTDIR\Uninstall ${APP}.exe"
  RMDir "$INSTDIR"

  Delete "$SMPROGRAMS\${APP}.lnk"
  Delete "$DESKTOP\${APP}.lnk"

  ReadRegStr $0 HKCU "${REGKEY}" "AssocStmacro"
  ${If} $0 == "1"
    ReadRegStr $1 HKCU "Software\Classes\.stmacro" ""
    ${If} $1 == "${PROGID}"
      DeleteRegKey HKCU "Software\Classes\.stmacro"
    ${EndIf}
    DeleteRegKey HKCU "Software\Classes\${PROGID}"
  ${EndIf}
  ReadRegStr $0 HKCU "${REGKEY}" "AssocRec"
  ${If} $0 == "1"
    DeleteRegValue HKCU "Software\Classes\.rec\OpenWithProgids" "${PROGID}.Rec"
    DeleteRegKey HKCU "Software\Classes\${PROGID}.Rec"
  ${EndIf}
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'

  DeleteRegKey HKCU "${UNINSTKEY}"
  DeleteRegKey HKCU "${REGKEY}"
  ; Your macros (Documents\SentinelTask) and settings (%APPDATA%\SentinelTask) are kept.
SectionEnd
