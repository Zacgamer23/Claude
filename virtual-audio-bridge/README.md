# Virtual Audio Bridge

Usa unos auriculares Bluetooth conectados al **iPhone** como si estuvieran conectados a un **PC Windows sin
Bluetooth**, a través de la Wi-Fi local. El móvil solo necesita **Safari**: no se instala ninguna app.

```
Apps de Windows ─► salida de Windows ─► [WASAPI loopback] ─► Opus ─► WebRTC/UDP ─► Safari ─► auriculares (HFP/A2DP)
Discord/OBS ◄── CABLE Output ◄── CABLE Input ◄── [WASAPI render] ◄── jitter buffer ◄── Opus ◄── WebRTC ◄── micrófono BT
```

## Estado

| Fase | Estado |
|---|---|
| 1. Viabilidad | Terminada → [docs/fase1-viabilidad.md](docs/fase1-viabilidad.md). Elegida la **opción C** (VB-CABLE para el micrófono virtual) |
| 2. Prueba de concepto | Código completo y pruebas automatizadas y simuladas superadas. **Falta la prueba con hardware real** (PC Windows + iPhone + Nothing Ear (3)) → [docs/pruebas.md](docs/pruebas.md) |
| 3–5 | Pendientes de los resultados de la Fase 2 |

## Estructura

| Ruta | Contenido |
|---|---|
| `engine/src/audio/` | Captura WASAPI loopback y entrega al micrófono virtual (hilos MMCSS, colas SPSC sin bloqueo); backend de prueba |
| `engine/src/dsp/` | Remuestreador, control de deriva de reloj (PI) y estimador de jitter |
| `engine/src/rtc.rs` | WebRTC (str0m, ICE-lite, sin STUN/TURN), Opus 10 ms + FEC, PLC |
| `engine/src/server/` | HTTPS para el móvil (página, emparejamiento, WSS) e interfaz local en 127.0.0.1 |
| `engine/src/auth.rs`, `tls.rs` | Token de un solo uso y sesiones; CA local y certificado con la IP en el SAN |
| `web/phone/` | Página del iPhone |
| `web/admin/` | Interfaz del PC |
| `installer/` | Instalador NSIS (firewall solo en red Privada y subred local; arranque automático) |
| `scripts/` | Compilación para Windows y diagnóstico |
| `tests/e2e.cjs` | Prueba extremo a extremo con Chromium (simulación) |

## Compilar

```bash
cd engine && cargo test                                         # pruebas unitarias
cargo build --features test-io && node ../tests/e2e.cjs         # prueba extremo a extremo (simulación)
../scripts/build-windows.sh                                     # vab-engine.exe + instalador en dist/
```

Instalación y uso: [docs/instalacion-y-uso.md](docs/instalacion-y-uso.md).
