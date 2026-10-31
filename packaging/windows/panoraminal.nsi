; Panoraminal installer (NSIS 3, Modern UI 2)
; Build: makensis -DVERSION=1.2.0 -DSRCDIR=<dir with panoraminal.exe> -DOUTFILE=<setup.exe> packaging/windows/panoraminal.nsi
; -DTARGET_AMD64 builds a native 64-bit installer (needs NSIS amd64 stubs, e.g. Debian/Ubuntu nsis).
!ifdef TARGET_AMD64
  Target amd64-unicode
!else
  Unicode true
!endif
!include "MUI2.nsh"
!include "FileFunc.nsh"

!ifndef VERSION
  !define VERSION "1.2.0"
!endif
!ifndef SRCDIR
  !define SRCDIR "..\..\target\x86_64-pc-windows-msvc\release"
!endif
!ifndef OUTFILE
  !define OUTFILE "Panoraminal-Setup-${VERSION}-x64.exe"
!endif

!define APP "Panoraminal"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Panoraminal"

Name "${APP} ${VERSION}"
OutFile "${OUTFILE}"
InstallDir "$PROGRAMFILES64\${APP}"
InstallDirRegKey HKLM "Software\${APP}" "InstallDir"
RequestExecutionLevel admin
SetCompressor /SOLID lzma
BrandingText "${APP} ${VERSION} — bugzinho.com"

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${APP}"
VIAddVersionKey "FileDescription" "${APP} installer"
VIAddVersionKey "CompanyName" "bugzinho"
VIAddVersionKey "LegalCopyright" "cleitinhodev / bugzinho — MIT License"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"

!define MUI_ICON "..\..\crates\panoraminal\assets\icons\panoraminal_icon.ico"
!define MUI_UNICON "..\..\crates\panoraminal\assets\icons\panoraminal_icon.ico"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\panoraminal.exe"
!define MUI_FINISHPAGE_LINK "www.bugzinho.com"
!define MUI_FINISHPAGE_LINK_LOCATION "https://www.bugzinho.com/"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "..\..\LICENSE"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "PortugueseBR"
!insertmacro MUI_LANGUAGE "Spanish"
!insertmacro MUI_LANGUAGE "French"
!insertmacro MUI_LANGUAGE "German"
!insertmacro MUI_LANGUAGE "Italian"
!insertmacro MUI_LANGUAGE "Japanese"
!insertmacro MUI_LANGUAGE "SimpChinese"

Function .onInit
  !insertmacro MUI_LANGDLL_DISPLAY
FunctionEnd

Section "Panoraminal" SecMain
  SectionIn RO
  SetShellVarContext all
  SetOutPath "$INSTDIR"
  File "${SRCDIR}\panoraminal.exe"
  File "/oname=LICENSE.txt" "..\..\LICENSE"
  File "/oname=OFL-NotoSansCJK.txt" "..\..\crates\panoraminal\assets\fonts\OFL.txt"
  WriteUninstaller "$INSTDIR\Uninstall.exe"

  CreateDirectory "$SMPROGRAMS\${APP}"
  CreateShortcut "$SMPROGRAMS\${APP}\${APP}.lnk" "$INSTDIR\panoraminal.exe"
  CreateShortcut "$SMPROGRAMS\${APP}\Uninstall ${APP}.lnk" "$INSTDIR\Uninstall.exe"
  CreateShortcut "$DESKTOP\${APP}.lnk" "$INSTDIR\panoraminal.exe"

  ; .pano file association
  WriteRegStr HKCR ".pano" "" "Panoraminal.Project"
  WriteRegStr HKCR "Panoraminal.Project" "" "Panoraminal project"
  WriteRegStr HKCR "Panoraminal.Project\DefaultIcon" "" "$INSTDIR\panoraminal.exe,0"
  WriteRegStr HKCR "Panoraminal.Project\shell\open\command" "" '"$INSTDIR\panoraminal.exe" "%1"'

  WriteRegStr HKLM "Software\${APP}" "InstallDir" "$INSTDIR"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayName" "${APP}"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "${UNINST_KEY}" "Publisher" "bugzinho"
  WriteRegStr HKLM "${UNINST_KEY}" "URLInfoAbout" "https://github.com/cleitinhodev/panoraminal"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\panoraminal.exe,0"
  WriteRegStr HKLM "${UNINST_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegDWORD HKLM "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKLM "${UNINST_KEY}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  WriteRegDWORD HKLM "${UNINST_KEY}" "EstimatedSize" $0
SectionEnd

Section "Uninstall"
  SetShellVarContext all
  Delete "$INSTDIR\panoraminal.exe"
  Delete "$INSTDIR\LICENSE.txt"
  Delete "$INSTDIR\OFL-NotoSansCJK.txt"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${APP}\${APP}.lnk"
  Delete "$SMPROGRAMS\${APP}\Uninstall ${APP}.lnk"
  RMDir "$SMPROGRAMS\${APP}"
  Delete "$DESKTOP\${APP}.lnk"
  DeleteRegKey HKCR "Panoraminal.Project"
  ReadRegStr $0 HKCR ".pano" ""
  StrCmp $0 "Panoraminal.Project" 0 +2
    DeleteRegKey HKCR ".pano"
  DeleteRegKey HKLM "${UNINST_KEY}"
  DeleteRegKey HKLM "Software\${APP}"
SectionEnd
