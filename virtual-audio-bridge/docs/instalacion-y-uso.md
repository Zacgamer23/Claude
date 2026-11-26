# Virtual Audio Bridge — instalación y uso

> **Estado: prueba de concepto (v0.1.0).** El transporte, la página web y el motor están probados de forma
> automatizada en Linux con Chromium (simulación, ver `pruebas.md`). **Todavía no se ha probado en Windows ni con
> un iPhone ni con auriculares reales.** Esta guía es también el protocolo de esa primera prueba.

## Qué necesitas

- PC con Windows 10 (22H2) u 11 de 64 bits, conectado a la Wi-Fi de casa.
- iPhone con Safari actualizado, en **la misma Wi-Fi**.
- Auriculares Bluetooth con micrófono (p. ej. Nothing Ear (3)) emparejados con el iPhone.

## 1. Instalar VB-CABLE (micrófono virtual)

VB-CABLE es un driver de audio gratuito y firmado de VB-Audio (opción «C» elegida en la Fase 1). Crea:
- **CABLE Input**: dispositivo de *reproducción*. Virtual Audio Bridge escribe aquí el audio de tu micrófono Bluetooth.
- **CABLE Output**: dispositivo de *grabación*. **Este es el micrófono que eliges en Discord, OBS, Zoom…**

1. Descárgalo de <https://vb-audio.com/Cable/> (botón *Download*, ZIP para Windows).
2. Descomprime el ZIP, clic derecho en `VBCABLE_Setup_x64.exe` → **Ejecutar como administrador** → *Install Driver*.
3. Reinicia el PC si lo pide.

## 2. Instalar Virtual Audio Bridge

1. Ejecuta `VirtualAudioBridge-0.1.0-setup.exe` y acepta el aviso de control de cuentas (necesario para crear las
   reglas del firewall).
   - **Windows SmartScreen** avisará de que la aplicación no está firmada (no tiene certificado de firma de código).
     Pulsa «Más información → Ejecutar de todas formas». Comprueba antes la huella SHA-256 en `SHA256SUMS.txt`.
2. Deja marcada la opción **Iniciar al arrancar Windows**: así el puente funciona aunque no abras la aplicación.
3. Al terminar se abre la interfaz en el navegador del PC (`http://127.0.0.1:47802`).

El instalador crea dos reglas del firewall **solo para redes Privadas y solo para la subred local**:
TCP 47800 (página y señalización) y UDP 47801 (audio WebRTC). La Wi-Fi de casa debe estar marcada como **Privada**
en Windows (Configuración → Red e Internet → Wi-Fi → tu red → Tipo de perfil de red → Privada).

## 3. Elegir los dispositivos en Windows

| Qué | Dónde | Elige |
|---|---|---|
| Salida de audio del PC (lo que oirás en los auriculares) | Configuración → Sistema → Sonido → Salida | La salida que vaya a capturar Virtual Audio Bridge (ver nota) |
| Micrófono en Discord/OBS/Zoom/juegos | Ajustes de audio de cada aplicación | **CABLE Output (VB-Audio Virtual Cable)** |

**Nota sobre la salida:** con la opción C solo hay un cable gratuito, y se usa para el micrófono. Por defecto,
Virtual Audio Bridge captura la **salida predeterminada de Windows** (por ejemplo, los altavoces o la salida de
auriculares de la tarjeta de sonido). En «Configuración avanzada» de la interfaz puedes elegir otra.
- No elijas **CABLE Input** como salida de Windows: es el camino del micrófono. La aplicación lo detecta y lo rechaza
  para no crear un bucle.
- Si tu salida predeterminada aparece como **desconectada** (por ejemplo, un conector de auriculares sin nada
  enchufado), Windows no permite capturarla. En ese caso hace falta un segundo dispositivo virtual de reproducción.
  Está documentado como pendiente en `pruebas.md`. **[A-PROBAR en tu PC]**

## 4. Conectar el iPhone

1. Conecta los auriculares al iPhone como siempre.
2. En la interfaz del PC, escanea el **código QR** con la cámara del iPhone y ábrelo en **Safari**.
3. Safari mostrará «Esta conexión no es privada», porque el certificado lo ha creado tu PC y no una autoridad pública.
   - Rápido: «Mostrar detalles» → «visitar este sitio web».
   - Recomendado (una sola vez, sin avisos después): en la página, despliega **Certificado de seguridad** y sigue los
     pasos para instalar el perfil y activar la confianza. Comprueba que la huella coincide con la de la interfaz del PC.
4. Elige el modo:
   - **Llamada**: micrófono de los auriculares + audio del PC a la vez. Bluetooth usa el perfil de llamada (HFP):
     audio mono de calidad de llamada. Esto lo impone el Bluetooth clásico, no la aplicación.
   - **Solo escucha**: audio estéreo de mayor calidad, sin micrófono.
5. Pulsa **Iniciar puente** y acepta el permiso de micrófono.
6. Si la página no puede saber qué micrófono está usando iOS, te pedirá que lo confirmes en el **Centro de control**
   (desliza desde la esquina superior derecha → icono de salida de audio → deben aparecer tus auriculares). Si iOS
   usa el micrófono interno del iPhone, la página **se niega a transmitirlo** y te lo dice.
7. **Mantén Safari en primer plano.** La página pide mantener la pantalla encendida; puedes bajar el brillo al mínimo.
   Lo que ocurre al bloquear la pantalla está pendiente de comprobar (ver limitaciones).

## 5. Interfaz del PC

Muestra datos reales del motor: estado del móvil y de WebRTC, el micrófono que usa el iPhone, los dispositivos de
Windows (nombre, estado, frecuencia y periodo), RTT de red, buffer de reproducción del iPhone, buffer y objetivo del
micrófono, deriva de reloj, pérdidas, audio reconstruido (PLC/FEC), cortes, niveles y el registro de errores.

Botones: **Iniciar/Detener puente**, **Desconectar móvil**, **Nuevo código**, **Revocar móviles** y **Salir**.
Para volver a abrir la interfaz con la aplicación ya en marcha, usa el acceso directo del menú Inicio.

## 6. Diagnóstico

```powershell
powershell -ExecutionPolicy Bypass -File diagnostico.ps1
```
Solo lee información (versión de Windows, Secure Boot, dispositivos de audio, VB-CABLE, perfil de red, firewall,
registro). Registro de la aplicación: `%APPDATA%\VirtualAudioBridge\vab.log`.

| Síntoma | Causa probable |
|---|---|
| El iPhone no abre la página | No están en la misma Wi-Fi, red de invitados con aislamiento de clientes, o la red de Windows es «Pública». |
| La página abre pero «Audio (WebRTC)» no conecta | Firewall bloqueando UDP 47801, o aislamiento de clientes en el router. |
| «Micrófono virtual: No se encuentra… CABLE Input» | VB-CABLE no está instalado (paso 1). |
| La página dice que usa el micrófono interno | iOS no ha enrutado la entrada a los auriculares: revisa el Centro de control o reconecta los auriculares. |
| Se corta al bloquear el iPhone | Restricción de iOS para páginas web (ver limitaciones). |

## 7. Desinstalar

1. Configuración → Aplicaciones → **Virtual Audio Bridge** → Desinstalar. Se quitan el programa, los accesos directos,
   el arranque automático y las reglas del firewall. Te pregunta si quieres borrar también la configuración y los
   certificados.
2. En el iPhone, si instalaste el perfil: Ajustes → General → VPN y gestión de dispositivos → Virtual Audio Bridge →
   Eliminar perfil.
3. VB-CABLE se desinstala ejecutando de nuevo su instalador como administrador → *Remove Driver*.

## Limitaciones conocidas

- **iPhone con la pantalla bloqueada o Safari en segundo plano:** según informes de WebKit, iOS puede suspender el
  micrófono y WebRTC. No está comprobado en tu iPhone; mientras tanto, mantén la página abierta.
- **Calidad en modo Llamada:** HFP es mono y de banda estrecha o ancha (8–16 kHz). Es una limitación física del
  Bluetooth clásico.
- **Latencia del Bluetooth:** el navegador no puede medirla. La latencia total se mide con la prueba de clic
  (`pruebas.md`).
- **Nombres de los dispositivos:** con la opción C se llaman «CABLE Input/Output» y no «Virtual Audio Bridge»
  (puedes renombrarlos en Panel de control → Sonido → Propiedades).
- **Ejecutable sin firma de código:** SmartScreen avisará. Firmarlo requiere un certificado de pago.
