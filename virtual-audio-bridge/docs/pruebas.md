# Pruebas — qué se ha comprobado y qué falta

Clasificación obligatoria de cada resultado:
- **AUTOMATIZADA**: prueba unitaria sin hardware.
- **SIMULACIÓN**: prueba extremo a extremo con navegador real, pero con audio sintético y sin dispositivos de Windows,
  iPhone ni Bluetooth.
- **HARDWARE REAL**: en tu PC Windows con iPhone y Nothing Ear (3). **Ninguna ejecutada todavía.**

## 1. Pruebas automatizadas (ejecutadas, 17/17 superadas — 2026-11-26)

`cd engine && cargo test`

| Módulo | Qué comprueba |
|---|---|
| `dsp::resample` | Relación 1,0 = identidad exacta; 44,1→48 kHz conserva el tono (1 kHz ± 1 Hz) y la amplitud; detención limpia sin datos y continuidad al reanudar |
| `dsp::drift` | Con desfases de reloj de −300 a +400 ppm, el buffer converge al objetivo (± 2 ms) y la corrección iguala el desfase (± 20 ppm); corrección limitada a ±1000 ppm |
| `dsp::jitter` | Red estable → objetivo mínimo; red con 0–40 ms de jitter → objetivo ~70 ms; al mejorar la red vuelve a bajar; límite máximo |
| `auth` | Token de un solo uso; bloqueo tras 20 intentos fallidos; revocación; sesiones persistentes tras reiniciar; aleatoriedad |
| `tls` | Cadena CA + servidor; reutilización; nuevo certificado si cambia la IP con la misma CA; base64 |
| `net` | Rangos privados; rechazo de IP pública configurada |

## 2. Prueba extremo a extremo (ejecutada, todas superadas — SIMULACIÓN)

`cd engine && cargo build --features test-io && cd .. && node tests/e2e.cjs`

Entorno: contenedor Linux, Chromium (Playwright) con micrófono falso, motor y navegador **en la misma máquina**
(127.0.0.1). El backend `test-io` sustituye WASAPI por un tono de 440 Hz y un medidor.

| Comprobación | Resultado |
|---|---|
| La API local exige token | ✓ |
| Emparejamiento con token de un solo uso, que se elimina de la URL | ✓ |
| Señalización WSS conectada | ✓ |
| Con un micrófono no identificable, la página exige confirmación y **no transmite** | ✓ |
| WebRTC conectado **sin STUN/TURN** (ICE-lite en el PC) | ✓ |
| PC → navegador: ~600 paquetes en 6 s, nivel 0,50 recibido (tono) | ✓ |
| Navegador → PC: ~300 paquetes, nivel 1,0 en el medidor (pitidos del micrófono falso) | ✓ |
| El PC recibe el informe del móvil (entrada, visibilidad, estadísticas) | ✓ |
| Reinicio de la app del PC → la página se reconecta sola, sin reemparejar | ✓ |
| «Desconectar móvil» desde el PC → la página se detiene y lo explica | ✓ |
| Un token ya usado se rechaza (403) | ✓ |

Medidas de esa simulación (misma máquina; **no representan la Wi-Fi ni el iPhone**): RTT 1–7 ms, buffer de recepción
de Chromium ~35 ms, buffer del micrófono en el PC ~40 ms (objetivo ~33 ms), 0 cortes, 0 PLC.

Fallos encontrados por estas pruebas y corregidos:
- Prueba del resampler con retardo mal supuesto (la prueba estaba mal, no el código).
- «Desconectar móvil» se mostraba en la página como «otra pestaña tomó el control».
- Las sesiones solo vivían en memoria: un reinicio del PC obligaba a reemparejar. Ahora se guardan (solo hashes).
- Tras reiniciar el PC, la página esperaba al timeout de ICE; ahora renegocia en cuanto el PC informa de que no tiene
  sesión.

## 3. Compilación para Windows (ejecutada)

`scripts/build-windows.sh` compila de forma cruzada con MinGW (x86_64-pc-windows-gnu) y genera el instalador NSIS.
El `.exe` solo importa DLLs del sistema (KERNEL32, ole32, avrt, ws2_32, bcrypt…). **No se ha ejecutado en Windows.**

## 4. Pruebas con hardware real (PENDIENTES)

Protocolo para el primer arranque. Anota la versión de iOS en cada prueba.

| # | Prueba | Cómo | Resultado |
|---|---|---|---|
| H1 | Instalación y arranque | Instalador; la interfaz abre; `diagnostico.ps1` | pendiente |
| H2 | Dispositivos | «Altavoz virtual» y «Micrófono virtual» activos en la interfaz con la frecuencia y el periodo reales | pendiente |
| H3 | Conexión del iPhone | QR → Safari → «Audio (WebRTC): conectado» | pendiente |
| H4 | Salida en los Nothing Ear (3) | Reproduce un vídeo en el PC y escúchalo en los auriculares | pendiente |
| H5 | Micrófono Bluetooth | Etiqueta de entrada en la página/interfaz; grabadora de Windows con «CABLE Output»; tapar el micrófono del iPhone no debe afectar | pendiente |
| H6 | Simultáneo | Llamada de prueba en Discord («Probar micrófono») mientras suena audio | pendiente |
| H7 | Aplicaciones | Discord, OBS, grabadora de Windows, un juego | pendiente |
| H8 | Estabilidad | 30 min de llamada: cortes, PLC, deriva en la interfaz | pendiente |
| H9 | Reconexión | Wi-Fi del iPhone off/on; reinicio del PC; Safari recargado | pendiente |
| H10 | Pantalla bloqueada / segundo plano | Bloquear 2 min; cambiar de app 1 min; anotar si sigue el audio en cada sentido | pendiente |
| H11 | Latencia total | Prueba de clic (abajo) | pendiente |
| H12 | Salida desconectada | Si la salida predeterminada del PC está «desconectada», comprobar si se puede capturar | pendiente |

### Prueba de clic (latencia acústica de ida y vuelta)

1. En el PC, abre Audacity: grabación desde **CABLE Output**, reproducción en la salida que captura el puente.
2. Reproduce un clic seco (generador «Click track») mientras graba.
3. Pon un auricular junto al micrófono del otro, o usa el micrófono para captar el propio auricular.
4. En la grabación, la distancia entre el clic original (en la pista generada) y su eco por el micrófono es la
   **latencia total de ida y vuelta**: PC → Wi-Fi → iPhone → Bluetooth → oído → micrófono → Bluetooth → iPhone →
   Wi-Fi → PC.
5. Réstale el *buffer del micrófono* y el RTT de red que muestra la interfaz para aislar la parte de iPhone + Bluetooth.
