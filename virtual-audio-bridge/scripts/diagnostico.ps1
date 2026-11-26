# Diagnóstico de Virtual Audio Bridge para Windows.
# Uso (PowerShell, no hace falta administrador):
#   powershell -ExecutionPolicy Bypass -File diagnostico.ps1
# Solo LEE información; no cambia nada del sistema. Copia la salida y compártela.

$ErrorActionPreference = 'Continue'
function Section($t) { Write-Host "`n=== $t ===" -ForegroundColor Cyan }

Section 'Windows'
$os = Get-CimInstance Win32_OperatingSystem
$cv = Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion'
"{0} {1} (compilación {2}.{3}), {4}" -f $os.Caption, $cv.DisplayVersion, $cv.CurrentBuild, $cv.UBR, $os.OSArchitecture

Section 'Arranque seguro e integridad de memoria'
try { $sb = Get-ItemPropertyValue 'HKLM:\SYSTEM\CurrentControlSet\Control\SecureBoot\State' -Name UEFISecureBootEnabled -ErrorAction Stop
      "Secure Boot: " + $(if ($sb -eq 1) { 'activado' } else { 'desactivado' }) } catch { 'Secure Boot: no disponible (BIOS heredada o sin permiso)' }
try { $h = Get-ItemPropertyValue 'HKLM:\SYSTEM\CurrentControlSet\Control\DeviceGuard\Scenarios\HypervisorEnforcedCodeIntegrity' -Name Enabled -ErrorAction Stop
      "Integridad de memoria (HVCI): " + $(if ($h -eq 1) { 'activada' } else { 'desactivada' }) } catch { 'Integridad de memoria (HVCI): no configurada' }
try { $ts = (bcdedit /enum '{current}' 2>$null | Select-String 'testsigning'); "Modo de pruebas de drivers: " + $(if ($ts -match 'Yes') { 'ACTIVADO' } else { 'desactivado' }) } catch {}

Section 'Bluetooth'
$bt = Get-PnpDevice -Class Bluetooth -ErrorAction SilentlyContinue | Where-Object Status -eq 'OK'
if ($bt) { 'Adaptadores Bluetooth: ' + ($bt.FriendlyName -join ', ') } else { 'Sin adaptador Bluetooth activo (esperado en este proyecto)' }

Section 'Dispositivos de audio (endpoints)'
Get-PnpDevice -Class AudioEndpoint -ErrorAction SilentlyContinue | Sort-Object FriendlyName |
  ForEach-Object { "{0,-8} {1}" -f $_.Status, $_.FriendlyName }

Section 'VB-CABLE'
$cable = Get-PnpDevice -Class AudioEndpoint -ErrorAction SilentlyContinue | Where-Object FriendlyName -match 'CABLE'
if ($cable) { 'VB-CABLE instalado: ' + ($cable.FriendlyName -join ' | ') }
else { 'VB-CABLE NO encontrado. Descárgalo de https://vb-audio.com/Cable/ (es gratuito) e instálalo como administrador.' }
$drv = Get-CimInstance Win32_PnPSignedDriver -ErrorAction SilentlyContinue | Where-Object { $_.DeviceName -match 'VB-Audio|CABLE' }
foreach ($d in $drv) { "  Driver: {0}  versión {1}  firmado por: {2}" -f $d.DeviceName, $d.DriverVersion, $d.Signer }

Section 'Formato del motor de audio'
'Comprueba en Panel de control → Sonido que "CABLE Input" y el dispositivo de salida usan 48000 Hz para evitar conversiones.'

Section 'Red'
Get-NetConnectionProfile | ForEach-Object { "Perfil '{0}' en {1}: {2}" -f $_.Name, $_.InterfaceAlias, $_.NetworkCategory }
Get-NetIPAddress -AddressFamily IPv4 -ErrorAction SilentlyContinue | Where-Object { $_.IPAddress -notlike '127.*' -and $_.IPAddress -notlike '169.254.*' } |
  ForEach-Object { "  {0,-16} {1}" -f $_.IPAddress, $_.InterfaceAlias }
if (Get-NetConnectionProfile | Where-Object NetworkCategory -eq 'Public') {
  Write-Host 'AVISO: hay una red marcada como Pública. Para la Wi-Fi de casa, márcala como Privada (Configuración → Red e Internet → Wi-Fi → propiedades).' -ForegroundColor Yellow }

Section 'Firewall'
Get-NetFirewallRule -DisplayName 'Virtual Audio Bridge*' -ErrorAction SilentlyContinue |
  ForEach-Object { "{0}: habilitada={1} perfil={2} acción={3}" -f $_.DisplayName, $_.Enabled, $_.Profile, $_.Action }
if (-not (Get-NetFirewallRule -DisplayName 'Virtual Audio Bridge*' -ErrorAction SilentlyContinue)) { 'Sin reglas de Virtual Audio Bridge (se crean al instalar).' }

Section 'Virtual Audio Bridge'
$exe = Join-Path ${env:ProgramFiles} 'Virtual Audio Bridge\vab-engine.exe'
if (Test-Path $exe) { "Instalado en $exe"; & $exe --list-devices } else { 'No instalado todavía.' }
$p = Get-Process vab-engine -ErrorAction SilentlyContinue
if ($p) { "En ejecución (PID $($p.Id))" } else { 'No está en ejecución.' }
foreach ($port in 47800, 47802) {
  $l = Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue
  if ($l) { "Puerto TCP $port escuchando en $($l.LocalAddress -join ', ')" } }
$log = Join-Path $env:APPDATA 'VirtualAudioBridge\vab.log'
if (Test-Path $log) { "`nÚltimas líneas del registro:"; Get-Content $log -Tail 25 }
