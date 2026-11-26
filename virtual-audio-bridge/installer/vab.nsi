; Instalador de Virtual Audio Bridge (NSIS).
; Compilar: scripts/build-windows.sh  (usa makensis; también funciona en Windows).
;
; Qué hace:
;  * Copia vab-engine.exe en "Archivos de programa\Virtual Audio Bridge".
;  * Crea accesos directos en el menú Inicio y (opcional) arranque al iniciar sesión.
;  * Crea dos reglas del Firewall de Windows SOLO para el perfil de red Privada y
;    SOLO para la subred local (TCP 47800 página/señalización, UDP 47801 audio).
;  * Registra el desinstalador en "Aplicaciones instaladas".
; No instala VB-CABLE: su licencia no permite redistribuirlo. Se instala aparte.

Unicode true
!define APPNAME "Virtual Audio Bridge"
!define EXE "vab-engine.exe"
!ifndef VERSION
  !define VERSION "0.1.0"
!endif
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\VirtualAudioBridge"

Name "${APPNAME}"
OutFile "..\dist\VirtualAudioBridge-${VERSION}-setup.exe"
InstallDir "$PROGRAMFILES64\${APPNAME}"
RequestExecutionLevel admin
SetCompressor /SOLID lzma

!include "MUI2.nsh"
!include "x64.nsh"

!define MUI_FINISHPAGE_RUN "$INSTDIR\${EXE}"
!define MUI_FINISHPAGE_RUN_TEXT "Abrir Virtual Audio Bridge"
!define MUI_FINISHPAGE_SHOWREADME "https://vb-audio.com/Cable/"
!define MUI_FINISHPAGE_SHOWREADME_TEXT "Descargar VB-CABLE (necesario para el micrófono virtual)"
!define MUI_FINISHPAGE_SHOWREADME_NOTCHECKED

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "Spanish"

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "Virtual Audio Bridge requiere Windows de 64 bits."
    Abort
  ${EndIf}
  SetRegView 64
FunctionEnd

Section "Virtual Audio Bridge (obligatorio)" SecMain
  SectionIn RO
  ; Cerrar una instancia en ejecución para poder sustituir el ejecutable.
  nsExec::Exec 'taskkill /IM ${EXE} /F'
  SetOutPath "$INSTDIR"
  File "..\dist\${EXE}"
  File "..\docs\instalacion-y-uso.md"
  File /oname=LICENSE.txt "..\LICENSE"

  CreateDirectory "$SMPROGRAMS\${APPNAME}"
  CreateShortcut "$SMPROGRAMS\${APPNAME}\${APPNAME}.lnk" "$INSTDIR\${EXE}"
  CreateShortcut "$SMPROGRAMS\${APPNAME}\Desinstalar.lnk" "$INSTDIR\uninstall.exe"

  ; Reglas del firewall: solo red Privada y subred local.
  nsExec::Exec 'netsh advfirewall firewall delete rule name="Virtual Audio Bridge (TCP)"'
  nsExec::Exec 'netsh advfirewall firewall delete rule name="Virtual Audio Bridge (UDP)"'
  nsExec::Exec 'netsh advfirewall firewall add rule name="Virtual Audio Bridge (TCP)" dir=in action=allow program="$INSTDIR\${EXE}" protocol=TCP localport=47800 profile=private remoteip=localsubnet'
  nsExec::Exec 'netsh advfirewall firewall add rule name="Virtual Audio Bridge (UDP)" dir=in action=allow program="$INSTDIR\${EXE}" protocol=UDP localport=47801 profile=private remoteip=localsubnet'

  WriteUninstaller "$INSTDIR\uninstall.exe"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayName" "${APPNAME}"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "${UNINST_KEY}" "Publisher" "Virtual Audio Bridge"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\${EXE}"
  WriteRegStr HKLM "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKLM "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegDWORD HKLM "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKLM "${UNINST_KEY}" "NoRepair" 1
SectionEnd

Section "Iniciar al arrancar Windows (recomendado)" SecAutostart
  ; Arranca en segundo plano al iniciar sesión, sin abrir la interfaz.
  CreateShortcut "$SMSTARTUP\${APPNAME}.lnk" "$INSTDIR\${EXE}" "--no-browser"
SectionEnd

Section "Uninstall"
  SetRegView 64
  nsExec::Exec 'taskkill /IM ${EXE} /F'
  nsExec::Exec 'netsh advfirewall firewall delete rule name="Virtual Audio Bridge (TCP)"'
  nsExec::Exec 'netsh advfirewall firewall delete rule name="Virtual Audio Bridge (UDP)"'
  Delete "$SMSTARTUP\${APPNAME}.lnk"
  Delete "$SMPROGRAMS\${APPNAME}\${APPNAME}.lnk"
  Delete "$SMPROGRAMS\${APPNAME}\Desinstalar.lnk"
  RMDir "$SMPROGRAMS\${APPNAME}"
  Delete "$INSTDIR\${EXE}"
  Delete "$INSTDIR\instalacion-y-uso.md"
  Delete "$INSTDIR\LICENSE.txt"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKLM "${UNINST_KEY}"
  MessageBox MB_YESNO "¿Borrar también la configuración, el registro y los certificados locales (%APPDATA%\Virtual Audio Bridge)?$\n$\nSi los borras, tendrás que volver a emparejar el iPhone e instalar el nuevo certificado." IDNO skipdata
    SetShellVarContext current
    RMDir /r "$APPDATA\VirtualAudioBridge"
  skipdata:
  MessageBox MB_ICONINFORMATION "Recuerda quitar el perfil del certificado del iPhone si lo instalaste:$\nAjustes → General → VPN y gestión de dispositivos → Virtual Audio Bridge → Eliminar."
SectionEnd

LangString DESC_Main ${LANG_SPANISH} "Motor de audio, servidor local y página para el móvil."
LangString DESC_Auto ${LANG_SPANISH} "Inicia Virtual Audio Bridge en segundo plano al iniciar sesión, para que el audio funcione sin abrir la aplicación."
!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecMain} $(DESC_Main)
  !insertmacro MUI_DESCRIPTION_TEXT ${SecAutostart} $(DESC_Auto)
!insertmacro MUI_FUNCTION_DESCRIPTION_END
