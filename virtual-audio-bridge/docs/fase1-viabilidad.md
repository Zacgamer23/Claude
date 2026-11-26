# Virtual Audio Bridge — Fase 1: viabilidad técnica y arquitectura

Fecha: 2026-11-26 · Estado: **investigación terminada; decisiones pendientes del usuario (§10)**

> Convención de este documento:
> - **[VERIFICADO-DOC]**: confirmado en documentación oficial o en el código fuente del proyecto citado.
> - **[A-PROBAR]**: plausible según las fuentes, pero solo se puede confirmar con hardware real (PoC, Fase 2).
> - **[NO-VIABLE]**: lo impide la plataforma; se explica por qué y qué alternativa hay.
>
> Ninguna afirmación de este documento procede de una prueba con hardware: **todavía no se ha ejecutado
> nada en Windows ni en un móvil** (ver §0).

---

## 0. Inspección del entorno de trabajo

| Elemento | Resultado |
|---|---|
| Entorno donde se ejecuta Claude Code | Contenedor Linux en la nube (Ubuntu 24.04, x86_64). **No es Windows.** |
| Equipo Windows del usuario conectado a la sesión | **Ninguno** (`list_computer_folders` devuelve una lista vacía). |
| Móvil / auriculares Bluetooth | No accesibles desde aquí. |
| Herramientas disponibles | Go, Rust/Cargo, Node 22, Python 3, CMake, Chromium + Playwright. |
| Herramientas no disponibles | Visual Studio, Windows Driver Kit (WDK), `signtool`, `inf2cat`, `pnputil`, WASAPI. |
| Contenido previo del repositorio | Un proyecto sin relación (generador de perfiles, `index.html`). **No se toca**; este proyecto va en `virtual-audio-bridge/`. |

Consecuencias:

1. **No puedo comprobar la versión de Windows, sus dispositivos de audio ni el aislamiento de la Wi-Fi.** Estos
   datos tienen que salir de tu PC (en §9 está el script de diagnóstico que hay que ejecutar).
2. **El driver de kernel no se puede compilar ni probar aquí.** El WDK solo funciona en Windows con Visual Studio.
3. Lo que **sí** se puede hacer y probar aquí, con pruebas automatizadas: la página web móvil, la señalización,
   la pila WebRTC del PC, el jitter buffer, los buffers circulares, el resampler, la autenticación y el
   servidor HTTPS. Para probar extremo a extremo en Linux se usa Chromium con un micrófono simulado.
   **Esto es una simulación, y así se indicará siempre.**

---

## 1. ¿Se pueden crear un altavoz y un micrófono virtuales reales en Windows?

### 1.1 ¿Sin driver de kernel? → **[NO-VIABLE]**

- El motor de audio de Windows (`audiodg` / AudioSrv) solo enumera los *endpoints* que exponen los drivers de
  kernel (PortCls/WaveRT, o ACX). Las APIs de modo usuario (WASAPI, MMDevice) **consumen** endpoints; no los crean.
  [VERIFICADO-DOC: Microsoft, *User-Mode Audio Components*]
- ACX (Audio Class Extensions), el modelo nuevo de Microsoft, está construido sobre **KMDF, no UMDF**: también es
  kernel. [VERIFICADO-DOC: *ACX audio class extensions overview*]
- Los APO (Audio Processing Objects) se ejecutan en modo usuario, pero solo procesan el audio de un endpoint que
  ya existe; no crean dispositivos.
- *WASAPI loopback* **no es un micrófono virtual**: permite que *nuestra* aplicación capture lo que suena en un
  altavoz, pero Discord no puede seleccionarlo como entrada. *Process loopback* (Windows 10 2004+) tampoco.

**Conclusión: los dos dispositivos exigen un driver de audio de kernel.** No hay alternativa en modo usuario
que cumpla el requisito de que Windows y cualquier aplicación los vean como dispositivos normales.

### 1.2 Opciones de driver

| Opción | Altavoz | Micrófono alimentado por nuestra app | Firma para Secure Boot | Coste | Nombre "Virtual Audio Bridge" |
|---|---|---|---|---|---|
| **A. Driver propio** derivado de SysVAD (MS-PL) / ACX | Sí | Sí (IOCTL + buffer compartido, latencia mínima) | Necesita **firma de atestación de Microsoft** (§1.3) | Certificado EV, ~250–600 USD/año | Sí |
| **B. Fork de VirtualDrivers/Virtual-Audio-Driver** (MIT + MS-PL, basado en SysVAD) | Sí | **No de serie**: no documenta ninguna API/IPC para meter PCM en el micrófono (el autor la ofrece como desarrollo a medida) | Su release 25.7.14 se anuncia "Signed" vía SignPath Foundation; **no verificado** que la firma sea de Microsoft (necesaria en kernel con Secure Boot). Un fork modificado necesita **firma nueva** | 0 € si SignPath acepta el proyecto y la firma sirve para kernel: **[A-PROBAR]** | Sí (en un fork) |
| **C. VB-CABLE** (VB-Audio, gratuito "donationware", ya firmado) | Un cable = 1 reproducción + 1 grabación | Sí: la app reproduce en "CABLE Input" y Discord graba de "CABLE Output" | Ya firmado por el fabricante | 0 € (1 cable). Cables adicionales A+B: de pago | No (se puede renombrar a mano en Sonido) |
| **D. Modo de pruebas** (`bcdedit /set testsigning on`) con driver propio | Sí | Sí | Exige **desactivar Secure Boot**; marca de agua en el escritorio; algunos antitrampas (Vanguard, FACEIT…) se niegan a funcionar | 0 € | Sí |

- **D queda descartada** para tu equipo de uso diario: obliga a desactivar una medida de seguridad, cosa que
  prohibiste. Solo sirve para **desarrollo dentro de una máquina virtual de pruebas**.
- **C** cubre bien el **micrófono**, pero con un solo cable gratuito no hay altavoz independiente. Además es
  software propietario: su licencia no permite meterlo en nuestro instalador, así que lo instalarías tú por
  separado. Los nombres no serían los que pediste.
- Un altavoz virtual **no** necesita bucle interno: nuestra app lo captura con *WASAPI loopback* (los drivers
  basados en SysVAD lo permiten). El **micrófono** sí necesita un camino para que nuestro PCM entre en el driver.

### 1.3 Firma de drivers (Windows 10 1607+ y Windows 11) [VERIFICADO-DOC]

- Con Secure Boot activado, un driver de kernel nuevo **solo se carga si lo ha firmado Microsoft**:
  - **Firma de atestación** (Partner Center / Hardware Dev Center). Requisito: una cuenta con un **certificado
    de firma de código EV**. Sirve para distribuirlo tú mismo (instalador propio), pero no por Windows Update.
  - **WHCP/HLK** (certificación completa), necesaria para Windows Update.
- Los certificados EV se emiten normalmente **a organizaciones registradas**, no a particulares. Esto puede
  impedir que lo obtengas como persona física. **[A-CONFIRMAR con la autoridad de certificación]**
- Según una respuesta comunitaria en Microsoft Q&A (no es documentación oficial), Azure Trusted Signing no sirve
  para la atestación de hardware.
- Windows 11 con *Integridad de memoria* (HVCI) exige además que el driver sea compatible con HVCI. SysVAD lo es
  si no se introducen patrones prohibidos. Se comprueba con la herramienta *Driver Verifier* y con el propio HVCI.

**Esta es la restricción más importante del proyecto.** Un driver propio de calidad profesional es técnicamente
viable, pero **distribuirlo firmado cuesta dinero (certificado EV)** y puede requerir una entidad jurídica.
Eso choca con "no depender de servicios de pago". **Lo tienes que decidir tú (§10, decisión 1).**

### 1.4 Diseño del driver propio (si se elige A o B-fork)

- Base: SysVAD (WaveRT, PortCls) con dos endpoints. Se recorta lo que no se usa (offload, keyword spotter,
  Bluetooth simulado, etc.).
  - `Virtual Audio Bridge — Altavoz` (render). Nuestra app lo captura con WASAPI loopback, así que el driver no
    necesita lógica adicional.
  - `Virtual Audio Bridge — Micrófono` (capture). El driver lee de un **buffer circular compartido**. Nuestra
    app lo rellena y lo mapea en su espacio mediante un IOCTL autenticado (solo el SID del servicio o del usuario).
    Sin datos (app cerrada o móvil desconectado) entrega **silencio**, nunca audio inventado, y se cuenta un
    *underrun* que la UI muestra.
- Formato nativo: 48 kHz, 16 bits PCM (el micrófono puede ser mono o estéreo). Así no hay conversiones: Opus en
  WebRTC trabaja a 48 kHz.
- Periodos: soporte de `IAudioClient3` con periodos cortos (objetivo 2,5–5 ms en modo compartido; medirlo).
- Instalación: `pnputil /add-driver vab.inf /install`, más la creación del nodo raíz con SwDevice/DevCon-like
  desde el instalador. Desinstalación: eliminar el dispositivo y `pnputil /delete-driver oemNN.inf /uninstall`.
- Los dispositivos existen aunque la app esté cerrada (son del sistema). Para que **lleve audio** debe correr el
  motor. Por eso el motor se instala como **proceso en segundo plano que arranca al iniciar sesión** (servicio de
  usuario/bandeja), independiente de la interfaz gráfica.

---

## 2. Navegador móvil: Bluetooth, entrada/salida simultánea y segundo plano

### 2.1 Lo que una página web puede y no puede hacer

- **No puede** usar Web Bluetooth para audio: Web Bluetooth es solo GATT (BLE). No da acceso a A2DP, HFP ni LE
  Audio, ni puede crear perfiles. **[NO-VIABLE como sustituto; no se usará]**
- Sí puede usar `getUserMedia` (micrófono) y reproducir audio con `<audio>` o WebAudio. El **sistema operativo**
  decide la ruta Bluetooth.
- Requiere **contexto seguro (HTTPS)**. `http://192.168.x.x` **no** permite `getUserMedia` (§4).

### 2.2 Android + Chrome (plataforma prioritaria)

- Chrome para Android enumera las entradas de audio y, al usar un micrófono Bluetooth, **cambia Android al modo
  "comunicación"**. Así el audio pasa de **A2DP a SCO (HFP)** y el micrófono del auricular se enruta bien.
  El commit de Chromium dice que se probó con auriculares Bluetooth clásicos en Android anteriores a S y en S+.
  [VERIFICADO-DOC: commit de Chromium *"android audio: Set communication mode when using Bluetooth microphone"*]
- Hay informes de auriculares concretos con los que Chrome Android no capturaba el micrófono (issue en
  `webrtc/samples`, sin fecha clara). **[A-PROBAR con tus auriculares]**
- **Consecuencia física inevitable (no es una limitación del software):** con auriculares Bluetooth *clásicos*, para
  usar el micrófono **la reproducción también pasa a HFP**. Eso significa:
  - Audio **mono**, de **16 kHz con mSBC (banda ancha)** o **8 kHz con CVSD** según los auriculares. Es calidad de
    llamada, no de música.
  - A cambio, **HFP tiene menos latencia** que A2DP (~10–40 ms frente a ~100–250 ms).
  - Con auriculares **LE Audio (LC3)** y un móvil compatible (Android 13+), el modo dúplex puede llegar a 32 kHz.
    **[A-PROBAR]**
  - Ocurre exactamente lo mismo si conectas esos auriculares directamente a un PC: es una limitación de los
    perfiles Bluetooth clásicos.
  - **Propuesta:** dos modos que el usuario elige y **nunca cambian solos**: *Llamada/dúplex* (HFP: micrófono +
    audio a la vez) y *Solo escucha* (A2DP: estéreo de alta calidad, sin micrófono). Tu requisito de no conmutar
    durante una conversación se cumple con el modo dúplex.
- **No se sustituirá el micrófono en silencio.** La página lista las entradas (`enumerateDevices`), selecciona la
  de Bluetooth y, tras abrirla, comprueba la etiqueta de la pista. Si la entrada activa no es Bluetooth, se
  **detiene y avisa** en lugar de transmitir el micrófono interno.
  **[A-PROBAR: Chrome Android debe exponer una etiqueta identificable como Bluetooth]**
- Salida: Chrome para Android no ofrece una selección de salida fiable (`setSinkId`). En modo comunicación, Android
  enruta la salida a SCO cuando el auricular está conectado. **[A-PROBAR]**
- **Segundo plano y pantalla apagada:** hay informes (W3C 2020; Chromium issue 513633) de que Chrome Android
  **corta micrófono y reproducción ~1 minuto después de apagar la pantalla** en llamadas WebRTC. No he encontrado
  confirmación de que esté resuelto en las versiones actuales. **[A-PROBAR; riesgo alto]**
  - Mitigaciones que no instalan nada: Screen Wake Lock API (mantiene la pantalla encendida, se puede atenuar al
    mínimo), Media Session API y excluir Chrome de la optimización de batería en Ajustes de Android (es un ajuste
    del sistema, no una instalación).
  - **No afirmaré que funciona con la pantalla apagada** hasta medirlo.
- Permisos necesarios: solo *Micrófono* para el origen `https://<IP-del-PC>:<puerto>`. No se piden cámara,
  ubicación, Bluetooth ni notificaciones.

### 2.3 iOS — **plataforma real del usuario** (iPhone + Nothing Ear (3)), actualizado 2026-11-26

El usuario no tiene Android; usa iPhone. En iOS **todos los navegadores (Safari, Chrome, Edge…) usan WebKit**, así
que las restricciones son las mismas en cualquiera de ellos. Una página web sigue siendo viable (no hay que
instalar nada), pero con estas restricciones:

- `getUserMedia` funciona en HTTPS. Al capturar el micrófono, iOS pasa a la sesión de audio *playAndRecord*, y con
  auriculares Bluetooth clásicos la ruta pasa a **HFP**. La reproducción también baja a calidad de llamada; el
  iPhone no usa LE Audio con auriculares de terceros. **[A-PROBAR con los Nothing Ear (3)]**
- **Selección del micrófono:** iOS decide la ruta; la página no puede elegirla de forma fiable. Hay informes
  (WebKit bug 174833, de iOS 11 a iOS 13, y un foro de octubre de 2025) de que `enumerateDevices()` solo lista
  "iPhone Microphone", aunque haya auriculares conectados. Un cambio de WebKit de diciembre de 2024 (bug 284617)
  deja de forzar un micrófono preferido para no interferir con el enrutado automático.
  → Es posible que la página **no pueda verificar por sí sola** que el micrófono activo es el de los auriculares.
  Para no sustituirlo en silencio: (a) se usa la etiqueta y el `deviceId` de la pista si iOS los expone, y
  (b) si no, la página **se detiene y pide confirmación explícita** de la ruta (Centro de control → salida de
  audio) antes de transmitir. **[A-PROBAR]**
- **Salida:** hay informes de que iOS manda el audio al altavoz del iPhone al activar el micrófono.
  `setSinkId` no está disponible en Safari iOS. Con auriculares Bluetooth conectados, lo esperado es la ruta HFP.
  **[A-PROBAR]**
- **Segundo plano / pantalla bloqueada:** hay varios informes (WebKit 226620, 241400; foros de Apple) de que iOS
  **suspende WebRTC y el micrófono al bloquear la pantalla o al pasar Safari a segundo plano**. Otro informe dice
  que una pestaña de Safari (no una web instalada en la pantalla de inicio) mantiene el micrófono. Las fuentes se
  contradicen y ninguna es oficial. Hipótesis de trabajo: **hay que mantener la página abierta en primer plano con
  la pantalla encendida** (Screen Wake Lock está disponible en Safari 16.4+). **[A-PROBAR en tu versión de iOS]**
- Si la PoC confirma que iOS corta el audio con la pantalla bloqueada, ese requisito **no se puede cumplir con una
  página web en iPhone**. La única alternativa sería una app nativa con modo de audio en segundo plano, que
  excluiste, así que no se hará sin tu permiso.
- Hay informes de fallos de `getUserMedia` en algunas versiones de iOS 26 (bug 319706; un fallo de la beta de
  iOS 26.1 se corrigió en la Beta 2). Conviene anotar la versión exacta de iOS en cada prueba.

### 2.4 Navegadores recomendados

1. **Chrome para Android** (objetivo principal).
2. Edge y Samsung Internet (Chromium) en Android: probablemente iguales. **[A-PROBAR]**
3. Firefox para Android: su enrutado de SCO es distinto. Secundario.
4. Safari en iOS: solo en primer plano.

---

## 3. Transporte de audio: comparación y elección

| Criterio | **WebRTC (Opus/RTP/SRTP sobre UDP)** | WebSocket + WebCodecs + AudioWorklet | WebTransport (QUIC datagramas) + WebCodecs |
|---|---|---|---|
| Transporte | UDP: un paquete perdido no bloquea a los siguientes | TCP: **bloqueo de cabeza de línea**; una retransmisión Wi-Fi congela todo el flujo | UDP (datagramas QUIC) |
| Jitter buffer adaptativo en el navegador | Sí (NetEQ), con ocultación de pérdidas; ajustable con `jitterBufferTarget` (Chrome 124+) | Hay que escribirlo en JS/Worklet | Hay que escribirlo |
| Códec | Opus integrado, FEC en banda, DTX, tramas de 10–20 ms | Opus vía WebCodecs (AudioEncoder) | Igual que WebSocket |
| Control de congestión / adaptación | Integrado (bitrate adaptativo, RTCP) | Ninguno | Parcial |
| Captura de micrófono en modo comunicación (necesario para SCO) | Sí: es el caso de uso nativo | `getUserMedia` igual, pero sin la pila de llamada | Igual |
| Requisitos de certificado | DTLS con huella intercambiada por señalización; **no necesita CA** | HTTPS/WSS de la página | Admite `serverCertificateHashes` (certificado ECDSA ≤14 días) |
| Estadísticas de latencia | `getStats()`: RTT, jitter, *jitterBufferDelay*, pérdidas, *concealedSamples* | Hay que implementarlas | Hay que implementarlas |
| Dependencias externas en LAN | Ninguna: candidatos *host* sin STUN/TURN | Ninguna | Ninguna |

**Elección: WebRTC** para el audio en ambos sentidos. Con WebSocket, los picos de retransmisión TCP de la Wi-Fi se
convierten en cortes o en latencia acumulada. WebTransport no aporta nada que WebRTC no tenga ya integrado, y
habría que reescribir el jitter buffer, la ocultación de pérdidas y las estadísticas.
El canal de **señalización** (SDP/ICE, control y estadísticas) sí va por **WebSocket seguro (WSS)** sobre el mismo
servidor HTTPS local. Es tráfico de control, no de audio.

### 3.1 Conexión WebRTC en LAN sin servidores externos

- Sin STUN ni TURN: `iceServers: []`.
- Chrome oculta su IP local tras candidatos **mDNS** (`xxxx.local`). No es un problema: el PC publica su candidato
  *host* (su IP LAN, UDP). El móvil le envía comprobaciones STUN y el PC aprende la dirección del móvil como
  candidato *peer-reflexive*. Para que funcione siempre, el PC actúa como agente **ICE-lite** o como agente
  completo que acepta *peer-reflexive*.
- La señalización viaja por WSS al mismo `https://IP:puerto` que sirve la página.
- **Aislamiento de clientes Wi-Fi (AP isolation):** si el router lo tiene activo (es habitual en redes de
  invitados), el móvil no puede alcanzar el PC ni siquiera por HTTPS, y la página no llega a cargar. La app de
  Windows lo diagnostica: comprueba que escucha en la interfaz correcta y que el Firewall de Windows tiene la regla
  para perfil "Privado". La UI explica la causa probable. **No se puede "saltar" sin cambiar el router o la red.**
- Firewall de Windows: el instalador crea reglas **solo para el perfil de red Privado y solo para la subred
  local** (TCP del servidor y UDP de ICE), en un rango de puertos fijo y configurable. Nada se expone a Internet
  ni a redes públicas.

### 3.2 Pila WebRTC en el PC

| Candidata | Ventajas | Inconvenientes |
|---|---|---|
| libwebrtc (Google, C++) | NetEQ completo, la referencia | Compilación enorme (gn/ninja, decenas de GB), mantenimiento costoso; trae captura de dispositivos que no queremos |
| **str0m** (Rust, *sans-IO*) | Sin hilos ni runtime propios, ICE-lite/completo, DTLS-SRTP, RTCP; control total del tiempo real; se usa en producción | El jitter buffer y Opus van por nuestra cuenta (libopus) |
| webrtc-rs (Rust) | API tipo navegador | Menos maduro; runtime async con más latencia impredecible |
| Pion (Go) | Muy maduro | El *garbage collector* de Go en la ruta de audio; Opus vía cgo |
| libdatachannel (C++) | Ligero | Igual que str0m en lo que falta; C++ menos seguro |

**Elección: Rust** para el motor (sin GC, crate `windows` para WASAPI y `DeviceIoControl`) con **str0m** para
WebRTC y **libopus** para el códec. Nuestro jitter buffer del lado PC es pequeño y adaptativo, con ocultación de
pérdidas mediante el PLC/FEC de Opus. La pila de libwebrtc no compensa su coste de compilación para un único
flujo de audio.

---

## 4. HTTPS y autenticación en red local (sin servicios externos)

- `getUserMedia` exige contexto seguro. `localhost` lo es, pero **la IP del PC vista desde el móvil no**.
  Opciones:
  1. **CA local propia** generada en la instalación (clave privada solo en el PC, protegida con DPAPI), que
     emite el certificado del servidor con la IP LAN en el SAN. El usuario **instala el certificado de la CA**
     en Android (Ajustes → Seguridad → Cifrado y credenciales → Instalar certificado CA). Chrome Android confía
     en las CA de usuario. Es una configuración del sistema, **no una app**, pero exige un paso manual y conviene
     que lo apruebes (§10, decisión 3).
  2. **Certificado autofirmado + aviso del navegador**: el usuario pulsa "Continuar" una vez. Chrome sigue
     tratando la página como HTTPS (contexto seguro) con indicador "No seguro". **[A-PROBAR que `getUserMedia`
     funciona así en Chrome Android]**. No desactiva ninguna medida de seguridad, pero es menos limpio.
  - **Descartado:** el flag `unsafely-treat-insecure-origin-as-secure` (desactiva una protección del navegador) y
    los servicios de DNS o certificados externos.
- **Autenticación:** el QR contiene `https://IP:puerto/#t=<token>`, con un token de 128 bits aleatorio (CSPRNG) y
  de **un solo uso**. El fragmento `#` no se envía en peticiones HTTP ni queda en registros. El token se canjea por
  una sesión ligada a ese dispositivo (cookie `HttpOnly; Secure; SameSite=Strict`).
  - Sin credenciales fijas en el código.
  - Un único móvil emparejado a la vez; la UI del PC muestra y permite revocar emparejamientos.
  - Límite de intentos y comprobación de `Origin` en WSS.
  - El QR incluye la huella SHA-256 del certificado para que el usuario pueda compararla.

---

## 5. Presupuesto de latencia (estimación, **no medición**)

Los valores son rangos esperados según la documentación y la experiencia típica de cada componente. Se
**sustituirán por mediciones** en la Fase 2/4.

### PC → auriculares (modo dúplex HFP)

| Etapa | Estimado | Tipo |
|---|---|---|
| Periodo del motor de audio de Windows + driver | 3–10 ms | Procesamiento |
| Captura WASAPI loopback (evento) | 3–10 ms | Captura |
| Trama Opus (10 ms) + codificación | 10–12 ms | Procesamiento |
| Wi-Fi LAN (sin ahorro de energía) | 2–10 ms (picos de 50–100+ ms si el móvil duerme la radio) | Transporte |
| Jitter buffer de Chrome (NetEQ) | 20–60 ms (`jitterBufferTarget` bajo) | Reproducción |
| Salida de audio Android (modo comunicación) | 10–40 ms | Reproducción |
| Bluetooth HFP (SCO) | 10–40 ms | Reproducción |
| **Total estimado** | **~60–180 ms** | |
| Con A2DP (modo "solo escucha") | +80–200 ms → **~150–350 ms** | |

### Micrófono → PC

| Etapa | Estimado |
|---|---|
| HFP SCO + captura Android | 20–50 ms |
| Trama Opus de Chrome (20 ms por defecto) + codificación | 20–25 ms |
| Wi-Fi | 2–10 ms |
| Jitter buffer del PC (nuestro, adaptativo) | 10–40 ms |
| Buffer circular del driver de micrófono + periodo del motor | 5–15 ms |
| **Total estimado** | **~60–140 ms** |

Los factores que más limitan la latencia **no dependen de nuestro código**: el Bluetooth (sobre todo A2DP), la
salida de audio de Android/Chrome y el ahorro de energía Wi-Fi del móvil. Se medirán por separado:
- **Transporte:** RTT de `getStats()`.
- **Jitter buffer:** `jitterBufferDelay / jitterBufferEmittedCount`.
- **Salida del navegador:** `AudioContext.outputLatency` cuando Chrome Android la reporta.
- **Total de ida y vuelta (acústica):** prueba de clic. El PC reproduce un pulso en el altavoz virtual, se acerca
  el micrófono del auricular a su altavoz y se mide cuándo vuelve por el micrófono virtual. Es la única medida
  real de extremo a extremo; se divide entre dos con cautela.

---

## 6. Motor de audio (diseño)

```
[Apps Windows] → Altavoz VAB (driver) → WASAPI loopback (hilo MMCSS "Pro Audio")
     → ring SPSC sin bloqueo → codificador Opus 10 ms → str0m (RTP/SRTP) → UDP → Chrome

Chrome (getUserMedia BT) → Opus → UDP → str0m → jitter buffer + PLC/FEC Opus
     → ring compartido con el driver (IOCTL) → Micrófono VAB (driver) → [Discord/OBS…]
```

- Hilos de audio con prioridad MMCSS, **sin asignaciones de memoria, sin locks y sin disco**. La comunicación con
  el resto usa colas SPSC preasignadas. El registro de eventos lo vacía otro hilo.
- 48 kHz de extremo a extremo. Solo se remuestrea si Windows fuerza otra frecuencia en el endpoint: resampler de
  calidad fija con deriva de reloj compensada (los relojes del PC y del móvil nunca coinciden, así que se ajusta
  lentamente la tasa para mantener constante el nivel del buffer y evitar cortes por acumulación).
- *Underrun* → silencio + contador; *overrun* → se descartan las muestras más antiguas + contador. Ambos se ven
  en la UI.
- El motor es un proceso independiente que arranca al iniciar sesión. La UI se conecta por un canal IPC local
  (named pipe con ACL del usuario) y no toca la ruta de audio.

---

## 7. Interfaz de Windows

Una UI ligera en un proceso separado (propuesta: **Tauri** con WebView2, que ya viene en Windows 10/11, o una UI
nativa en Rust). Muestra solo datos que el motor reporta por IPC: estado del driver (endpoints presentes y
activos, consultados con MMDevice), URL y QR, móvil emparejado, estado ICE/DTLS, RTT, jitter, buffers,
frecuencia, cortes, el estado de la entrada Bluetooth según la página móvil y el registro de errores.
"Estado de los auriculares": la web **solo** puede saber la etiqueta del dispositivo de entrada activo. La
batería o el códec Bluetooth **no son accesibles** desde un navegador. **[NO-VIABLE: se indicará así en la UI]**

---

## 8. Ratones y teclados Bluetooth (futuro)

- Una página web **sí** recibe los eventos de un teclado o ratón Bluetooth conectados al móvil (`keydown`,
  `pointermove`…), pero **solo con la página en primer plano y con foco**. Android intercepta algunas teclas
  (Inicio, Atrás, atajos del sistema) y el ratón mueve también el puntero del propio móvil. Pointer Lock en Chrome
  Android: **[A-PROBAR]**.
- En Windows, la inyección puede hacerse:
  - con `SendInput` (modo usuario, sin driver; no funciona en el escritorio seguro: UAC o pantalla de inicio de
    sesión);
  - o con un HID virtual (VHF, kernel, con el mismo problema de firma que el audio).
- Transporte: un *DataChannel* WebRTC no fiable y ordenado, independiente del audio, para no afectar a su latencia.
- **Viabilidad: parcial** (pantalla encendida, página activa). Queda documentado y con el canal previsto, pero **no
  se implementa ahora**: tiene prioridad el audio.

---

## 9. Hardware y datos necesarios para la Fase 2 (PoC real)

1. **PC Windows 10 22H2 o Windows 11 x64** (el tuyo).
2. **Máquina de desarrollo de drivers:** Visual Studio 2022 + WDK, idealmente con una **VM Hyper-V** en modo de
   pruebas para desarrollar el driver sin desactivar Secure Boot en tu equipo real.
3. **Móvil Android** con Chrome actualizado (indica el modelo y la versión de Android).
4. **Auriculares Bluetooth con micrófono** (indica el modelo; conviene saber si admiten mSBC o LE Audio).
5. La misma Wi-Fi, **sin aislamiento de clientes**.
6. Datos de tu PC (los recogerá un script `diagnostico.ps1` en la Fase 2): versión exacta de Windows, estado de
   Secure Boot e HVCI, dispositivos de audio actuales, perfil de red, IP y reglas de firewall.

Como este entorno no tiene acceso a tu PC ni a tu móvil, **las pruebas con hardware las tendrás que ejecutar tú**
con los scripts e instrucciones que prepararé. O bien puedes conectar tu PC a esta sesión con Remote Control
desde la app de escritorio de Claude.

---

## 10. Requisitos: qué se cumple y qué necesita una decisión

| Requisito | Estado |
|---|---|
| PC sin Bluetooth, conexión solo por Wi-Fi LAN | Viable |
| Nada instalado en el móvil (solo navegador) | Viable (con la nota de §4 sobre el certificado) |
| Sin servidores externos ni servicios de pago para transportar el audio | Viable (WebRTC en LAN, sin STUN/TURN) |
| Audio bidireccional simultáneo con el micrófono Bluetooth | Viable en Android/Chrome **[A-PROBAR]**, **en HFP** (calidad de llamada) |
| Altavoz y micrófono virtuales reales, seleccionables en Discord/OBS | Viable **solo con driver de kernel** |
| Driver firmado sin desactivar seguridad **y** sin pagar | **Conflicto** → decisión 1 |
| Funcionar con la pantalla apagada o en segundo plano | Incierto en Chrome Android; no en iOS **[A-PROBAR]** |
| Estado detallado de los auriculares | No disponible desde la web (solo la etiqueta del dispositivo) |
| Ratón/teclado Bluetooth | Parcial, aplazado |

### Decisiones que necesito de ti antes de la Fase 2/3

1. **Driver (bloqueante).** Elige una:
   - **A** – Driver propio "Virtual Audio Bridge" (la mejor calidad y latencia). Para tu PC real necesita **firma
     de atestación de Microsoft**: certificado EV (~250–600 USD/año, normalmente a nombre de una empresa) más una
     cuenta de Partner Center. El desarrollo se haría en una VM en modo de pruebas.
   - **B** – Lo mismo, partiendo del fork MIT de Virtual-Audio-Driver, e **intentar la firma gratuita de SignPath
     Foundation**. Antes hay que verificar que su firma carga en kernel con Secure Boot (sin garantía).
   - **C** – Usar **VB-CABLE** (gratuito, ya firmado; lo instalas tú) como micrófono virtual. Como altavoz se usa
     un segundo endpoint ya firmado (por ejemplo, el driver firmado de VirtualDrivers si se confirma su firma).
     Los nombres no serían "Virtual Audio Bridge", pero funciona sin pagar y sin desactivar Secure Boot.
     Cambia tu requisito de "driver propio con esos nombres".
   - Mi recomendación: **empezar por C** para tener el sistema completo funcionando y medido. El motor y la web no
     dependen del driver: el código abstrae el destino del micrófono (`DeviceIoControl` frente a un endpoint
     WASAPI). En paralelo, desarrollar **A/B** y cambiar a él cuando haya firma.
2. **Calidad en modo dúplex:** ¿aceptas que, con el micrófono activo, el audio de los auriculares sea mono a
   16 kHz (HFP)? Es una limitación del Bluetooth clásico, no del proyecto. Puedo añadir el modo "solo escucha"
   estéreo (A2DP) que eliges manualmente.
3. **Certificado HTTPS en el móvil:** ¿prefieres instalar una vez el certificado CA local en Android (sin avisos
   después) o aceptar el aviso del navegador cada vez que cambie el certificado?
4. **Pantalla apagada:** si en la PoC Chrome corta el audio con la pantalla apagada, ¿te vale mantener la pantalla
   encendida y atenuada (Wake Lock)? La otra opción es una app nativa, que excluiste y no haré sin tu permiso.

---

## Fuentes

- Microsoft — SysVAD sample: https://learn.microsoft.com/en-us/samples/microsoft/windows-driver-samples/sysvad-virtual-audio-device-driver-sample
- Microsoft — ACX overview: https://learn.microsoft.com/en-gb/windows-hardware/drivers/audio/acx-audio-class-extensions-overview
- Microsoft — User-mode audio components: https://learn.microsoft.com/en-ie/Windows/Win32/coreaudio/user-mode-audio-components
- Microsoft — Simple Audio Sample (firma necesaria): https://learn.microsoft.com/en-us/samples/microsoft/windows-driver-samples/simple-audio-sample-device-driver
- Microsoft — Driver code signing requirements: https://learn.microsoft.com/pl-pl/windows-hardware/drivers/dashboard/code-signing-reqs
- Microsoft Q&A — Trusted Signing y atestación: https://learn.microsoft.com/en-au/answers/questions/5866910/hardware-program-verification-using-azures-trusted
- OSR — virtual sound device en modo usuario: https://community.osr.com/t/advice-on-writing-a-virtual-sound-device-driver/58298
- VirtualDrivers/Virtual-Audio-Driver: https://github.com/VirtualDrivers/Virtual-Audio-Driver · releases: https://github.com/VirtualDrivers/Virtual-Audio-Driver/releases
- Chromium — modo comunicación con micrófono Bluetooth: https://chromium.googlesource.com/chromium/src/+/78d0de138346e6edfc8c33bdd22c45b3eec4f3d0
- webrtc/samples issue 1338 (fallo con auriculares concretos): https://www.github.com/webrtc/samples/issues/1338
- Mozilla bug 1091417 (restricciones de SCO): https://bugzilla.mozilla.org/show_bug.cgi?id=1091417
- W3C webrtc logs 2020 (corte con pantalla apagada en Chrome Android): https://lists.w3.org/Archives/Public/public-webrtc-logs/2020Mar/0129.html
- WebKit bug 226620 (micrófono en segundo plano, iOS): https://bugs.webkit.org/show_bug.cgi?id=226620
- MDN — `jitterBufferTarget`: https://developer.mozilla.org/en-US/docs/Web/API/RTCRtpReceiver/jitterBufferTarget
- Chromium Intent to Ship `jitterBufferTarget`: https://groups.google.com/a/chromium.org/g/blink-dev/c/bReU8otUmdk
