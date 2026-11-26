//! Estimador del jitter de llegada y del objetivo del jitter buffer.
//!
//! Para cada paquete se compara el instante de llegada con su marca de tiempo
//! RTP: `d = llegada - rtp`. La dispersión de `d` es el jitter. El objetivo del
//! buffer es el percentil 98 de `d - min(d)` en una ventana reciente, más un
//! margen de un periodo del dispositivo, limitado a [min, max]. Así el buffer
//! crece solo cuando la red lo exige y vuelve a bajar cuando mejora.

pub struct JitterEstimator {
    window: Vec<f64>,
    pos: usize,
    filled: usize,
    scratch: Vec<f64>,
    base: Option<f64>,
    rfc_jitter: f64,
    last_transit: Option<f64>,
    min_target: f64,
    max_target: f64,
}

impl JitterEstimator {
    /// `window_len`: nº de paquetes de la ventana (p. ej. 250 ≈ 5 s a 20 ms).
    pub fn new(window_len: usize, min_target: f64, max_target: f64) -> Self {
        Self {
            window: vec![0.0; window_len],
            pos: 0,
            filled: 0,
            scratch: vec![0.0; window_len],
            base: None,
            rfc_jitter: 0.0,
            last_transit: None,
            min_target,
            max_target,
        }
    }

    pub fn reset(&mut self) {
        self.pos = 0;
        self.filled = 0;
        self.base = None;
        self.rfc_jitter = 0.0;
        self.last_transit = None;
    }

    /// `arrival_s`: instante de llegada (s, reloj monótono local).
    /// `media_s`: marca de tiempo RTP convertida a segundos.
    pub fn on_packet(&mut self, arrival_s: f64, media_s: f64) {
        let transit = arrival_s - media_s;
        // Jitter RFC 3550 (para mostrar).
        if let Some(prev) = self.last_transit {
            let dd = (transit - prev).abs();
            self.rfc_jitter += (dd - self.rfc_jitter) / 16.0;
        }
        self.last_transit = Some(transit);
        // Referencia relativa para mantener valores pequeños.
        let base = *self.base.get_or_insert(transit);
        self.window[self.pos] = transit - base;
        self.pos = (self.pos + 1) % self.window.len();
        self.filled = (self.filled + 1).min(self.window.len());
    }

    pub fn jitter_s(&self) -> f64 {
        self.rfc_jitter
    }

    /// Objetivo del buffer en segundos. `frame_s` es la duración de un paquete
    /// (el buffer debe poder cubrir al menos uno completo) y `period_s` la del
    /// periodo del dispositivo de salida.
    pub fn target_s(&mut self, frame_s: f64, period_s: f64) -> f64 {
        let floor = (frame_s + period_s).max(self.min_target);
        if self.filled < 8 {
            return floor.max(0.040).min(self.max_target);
        }
        let n = self.filled;
        self.scratch[..n].copy_from_slice(&self.window[..n]);
        let s = &mut self.scratch[..n];
        s.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
        let min = s[0];
        let p98 = s[((n - 1) as f64 * 0.98) as usize];
        let spread = p98 - min;
        (floor + spread).clamp(self.min_target, self.max_target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steady_network_gives_small_target() {
        let mut j = JitterEstimator::new(250, 0.010, 0.200);
        for i in 0..500 {
            let t = i as f64 * 0.020;
            j.on_packet(t + 0.005, t);
        }
        let tgt = j.target_s(0.020, 0.010);
        assert!((tgt - 0.030).abs() < 1e-9, "tgt={tgt}");
        assert!(j.jitter_s() < 1e-9);
    }

    #[test]
    fn jittery_network_grows_target_and_recovers() {
        let mut j = JitterEstimator::new(250, 0.010, 0.200);
        // Retrasos de 0–40 ms pseudoaleatorios.
        let mut x: u32 = 12345;
        for i in 0..500 {
            x = x.wrapping_mul(1103515245).wrapping_add(12345);
            let delay = (x >> 16) as f64 / 65536.0 * 0.040;
            let t = i as f64 * 0.020;
            j.on_packet(t + delay, t);
        }
        let high = j.target_s(0.020, 0.010);
        assert!(high > 0.060 && high < 0.075, "high={high}");
        for i in 500..1000 {
            let t = i as f64 * 0.020;
            j.on_packet(t + 0.002, t);
        }
        let low = j.target_s(0.020, 0.010);
        assert!(low < 0.035, "low={low}");
    }

    #[test]
    fn respects_max() {
        let mut j = JitterEstimator::new(50, 0.010, 0.100);
        for i in 0..50 {
            let t = i as f64 * 0.02;
            j.on_packet(t + if i % 2 == 0 { 0.0 } else { 0.5 }, t);
        }
        assert_eq!(j.target_s(0.02, 0.01), 0.100);
    }
}
