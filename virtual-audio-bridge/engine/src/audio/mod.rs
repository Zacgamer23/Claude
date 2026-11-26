//! Motor de audio: hilos de captura (altavoz virtual → red) y de entrega
//! (red → micrófono virtual), conectados con el hilo WebRTC mediante colas SPSC
//! sin bloqueo (`rtrb`).
//!
//! Formato interno: 48 kHz, f32. Subida (PC→móvil) estéreo entrelazado; bajada
//! (móvil→PC) mono. 48 kHz es la frecuencia nativa de Opus: si el dispositivo de
//! Windows también está a 48 kHz no hay ninguna conversión de frecuencia.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::thread::JoinHandle;

use crate::stats::Stats;

#[cfg(windows)]
mod wasapi_backend;
#[cfg(feature = "test-io")]
mod test_backend;

pub const RATE: u32 = 48_000;
pub const UP_CHANNELS: usize = 2;

/// Capacidad de las colas: 1 s. El nivel real lo controla el consumidor; esta
/// capacidad solo es el límite de desbordamiento.
const RING_SECONDS: usize = 1;

/// Estado de control compartido con los hilos de audio (solo atómicos).
pub struct Control {
    /// El puente está activo (si no, la captura se descarta y el micrófono da silencio).
    pub bridge_enabled: AtomicBool,
    /// Objetivo del jitter buffer de bajada (µs), lo fija el hilo WebRTC.
    pub down_target_us: AtomicU64,
    /// Señal de parada de los hilos de audio actuales.
    pub stop: AtomicBool,
}

impl Default for Control {
    fn default() -> Self {
        Self {
            bridge_enabled: AtomicBool::new(true),
            down_target_us: AtomicU64::new(40_000),
            stop: AtomicBool::new(false),
        }
    }
}

/// Extremos de las colas que pertenecen al hilo WebRTC.
pub struct NetEnds {
    /// Audio capturado del altavoz virtual (estéreo entrelazado, 48 kHz).
    pub up: rtrb::Consumer<f32>,
    /// Audio decodificado del micrófono del móvil (mono, 48 kHz).
    pub down: rtrb::Producer<f32>,
}

/// Extremos de las colas que pertenecen a los hilos de dispositivo.
#[cfg_attr(not(any(windows, feature = "test-io")), allow(dead_code))]
pub struct DeviceEnds {
    pub up: rtrb::Producer<f32>,
    pub down: rtrb::Consumer<f32>,
}

pub fn make_pipes() -> (NetEnds, DeviceEnds) {
    let (up_p, up_c) = rtrb::RingBuffer::new(RATE as usize * UP_CHANNELS * RING_SECONDS);
    let (down_p, down_c) = rtrb::RingBuffer::new(RATE as usize * RING_SECONDS);
    (NetEnds { up: up_c, down: down_p }, DeviceEnds { up: up_p, down: down_c })
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub state: String,
    pub is_default: bool,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct DeviceStatus {
    /// Dispositivo de reproducción que se captura (el "altavoz virtual").
    pub speaker: Option<DeviceInfo>,
    /// Dispositivo de reproducción en el que se escribe el micrófono
    /// (p. ej. "CABLE Input"; las aplicaciones graban de "CABLE Output").
    pub mic_sink: Option<DeviceInfo>,
    pub speaker_error: Option<String>,
    pub mic_error: Option<String>,
}

/// Hilos de audio en marcha. Al soltarlo se detienen y se esperan.
pub struct AudioThreads {
    control: Arc<Control>,
    handles: Vec<JoinHandle<()>>,
}

impl AudioThreads {
    pub fn stop(mut self) {
        self.shutdown();
    }
    fn shutdown(&mut self) {
        self.control.stop.store(true, Relaxed);
        for h in self.handles.drain(..) {
            let _ = h.join();
        }
    }
}

impl Drop for AudioThreads {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Lista los dispositivos de reproducción de Windows.
pub fn list_render_devices() -> Result<Vec<DeviceInfo>, String> {
    #[cfg(windows)]
    {
        wasapi_backend::list_render_devices()
    }
    #[cfg(not(windows))]
    {
        Err("Los dispositivos de audio de Windows solo están disponibles en Windows".into())
    }
}

/// Arranca los hilos de dispositivo. `speaker` y `mic_sink` son subcadenas del
/// nombre del dispositivo (vacío = dispositivo de reproducción predeterminado
/// para el altavoz; obligatorio para el micrófono).
pub fn start(
    speaker: &str,
    mic_sink: &str,
    ends: DeviceEnds,
    control: Arc<Control>,
    stats: Arc<Stats>,
    log: crate::log::Log,
) -> (AudioThreads, DeviceStatus) {
    control.stop.store(false, Relaxed);
    #[cfg(feature = "test-io")]
    {
        let _ = (speaker, mic_sink);
        let (handles, status) = test_backend::start(ends, control.clone(), stats, log);
        (AudioThreads { control, handles }, status)
    }
    #[cfg(all(windows, not(feature = "test-io")))]
    {
        let (handles, status) = wasapi_backend::start(speaker, mic_sink, ends, control.clone(), stats, log);
        (AudioThreads { control, handles }, status)
    }
    #[cfg(all(not(windows), not(feature = "test-io")))]
    {
        let _ = (speaker, mic_sink, ends, stats);
        let msg = "Los dispositivos de audio de Windows solo están disponibles en Windows".to_string();
        log.error(&msg);
        (
            AudioThreads { control, handles: vec![] },
            DeviceStatus { speaker_error: Some(msg.clone()), mic_error: Some(msg), ..Default::default() },
        )
    }
}

/// Convierte un bloque entrelazado de `ch` canales a estéreo.
#[cfg_attr(not(windows), allow(dead_code))]
#[inline]
pub fn frame_to_stereo(frame: &[f32]) -> [f32; 2] {
    match frame.len() {
        0 => [0.0, 0.0],
        1 => [frame[0], frame[0]],
        _ => [frame[0], frame[1]],
    }
}
