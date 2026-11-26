//! Estadísticas compartidas entre los hilos de tiempo real y la interfaz.
//!
//! Los hilos de audio solo hacen operaciones atómicas (`Relaxed`) sobre estos
//! contadores: sin bloqueos ni asignaciones. La interfaz los lee periódicamente.

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering::Relaxed};

#[derive(Default)]
pub struct Stats {
    // --- PC → móvil (captura del altavoz virtual) ---
    pub capture_running: AtomicBool,
    pub capture_rate: AtomicU64,
    pub capture_period_us: AtomicU64,
    pub capture_frames: AtomicU64,
    pub capture_overruns: AtomicU64,
    pub tx_packets: AtomicU64,
    pub tx_bytes: AtomicU64,

    // --- móvil → PC (entrega al micrófono virtual) ---
    pub render_running: AtomicBool,
    pub render_rate: AtomicU64,
    pub render_period_us: AtomicU64,
    pub rx_packets: AtomicU64,
    pub rx_bytes: AtomicU64,
    pub rx_plc_frames: AtomicU64,
    pub rx_decode_errors: AtomicU64,
    pub render_underruns: AtomicU64,
    pub render_overruns: AtomicU64,
    /// Nivel actual del buffer de recepción en microsegundos.
    pub rx_buffer_us: AtomicU64,
    /// Objetivo actual del jitter buffer adaptativo en microsegundos.
    pub rx_target_us: AtomicU64,
    /// Jitter de llegada medido (microsegundos).
    pub rx_jitter_us: AtomicU64,
    /// Ajuste de deriva de reloj aplicado (partes por millón, con signo).
    pub drift_ppm: AtomicI64,
    /// Pico de nivel recibido desde la última lectura (0..=10000).
    pub rx_peak: AtomicU64,
    /// Pico de nivel capturado desde la última lectura (0..=10000).
    pub tx_peak: AtomicU64,

    // --- transporte ---
    pub peer_connected: AtomicBool,
    pub rtt_us: AtomicU64,
    pub ingress_loss_permille: AtomicU64,
    pub egress_loss_permille: AtomicU64,
}

impl Stats {
    pub fn snapshot(&self) -> serde_json::Value {
        let g = |a: &AtomicU64| a.load(Relaxed);
        serde_json::json!({
            "capture": {
                "running": self.capture_running.load(Relaxed),
                "rate": g(&self.capture_rate),
                "period_us": g(&self.capture_period_us),
                "frames": g(&self.capture_frames),
                "overruns": g(&self.capture_overruns),
                "peak": self.tx_peak.swap(0, Relaxed) as f64 / 10000.0,
            },
            "tx": { "packets": g(&self.tx_packets), "bytes": g(&self.tx_bytes) },
            "render": {
                "running": self.render_running.load(Relaxed),
                "rate": g(&self.render_rate),
                "period_us": g(&self.render_period_us),
                "underruns": g(&self.render_underruns),
                "overruns": g(&self.render_overruns),
                "buffer_ms": g(&self.rx_buffer_us) as f64 / 1000.0,
                "target_ms": g(&self.rx_target_us) as f64 / 1000.0,
                "drift_ppm": self.drift_ppm.load(Relaxed),
                "peak": self.rx_peak.swap(0, Relaxed) as f64 / 10000.0,
            },
            "rx": {
                "packets": g(&self.rx_packets),
                "bytes": g(&self.rx_bytes),
                "plc_frames": g(&self.rx_plc_frames),
                "decode_errors": g(&self.rx_decode_errors),
                "jitter_ms": g(&self.rx_jitter_us) as f64 / 1000.0,
            },
            "net": {
                "connected": self.peer_connected.load(Relaxed),
                "rtt_ms": g(&self.rtt_us) as f64 / 1000.0,
                "ingress_loss_pct": g(&self.ingress_loss_permille) as f64 / 10.0,
                "egress_loss_pct": g(&self.egress_loss_permille) as f64 / 10.0,
            },
        })
    }
}

/// Pico absoluto de un bloque, escalado a 0..=10000, para medidores de nivel.
#[inline]
pub fn peak_10k(samples: &[f32]) -> u64 {
    let mut p = 0.0f32;
    for &s in samples {
        let a = s.abs();
        if a > p {
            p = a;
        }
    }
    (p.min(1.0) * 10000.0) as u64
}
