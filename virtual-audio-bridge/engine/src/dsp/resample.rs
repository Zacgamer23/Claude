//! Remuestreador fraccional por interpolación cúbica (Hermite / Catmull-Rom).
//!
//! Se usa para dos cosas:
//! 1. Convertir entre 48 kHz (Opus/WebRTC) y la frecuencia del dispositivo de
//!    Windows cuando no coinciden (si coinciden, la relación es 1,0 exacta y la
//!    interpolación devuelve las muestras originales).
//! 2. Compensar la deriva entre el reloj del móvil y el del dispositivo de
//!    Windows, variando la relación unas pocas ppm (ver `drift`).
//!
//! La calidad es suficiente para voz de banda ancha y para corrección de deriva;
//! para conversiones de frecuencia grandes con música, la documentación indica
//! configurar el dispositivo de Windows a 48 kHz para que no haya conversión.

/// Remuestreador entrelazado de `CH` canales.
pub struct Resampler<const CH: usize> {
    /// Relación entrada/salida: muestras de entrada consumidas por muestra de salida.
    step: f64,
    /// Posición fraccional entre `hist[1]` y `hist[2]`.
    frac: f64,
    /// Historia de 4 tramas: x[-1], x[0], x[1], x[2].
    hist: [[f32; CH]; 4],
    /// Tramas de historia válidas (las primeras se rellenan con ceros).
    primed: usize,
}

impl<const CH: usize> Resampler<CH> {
    pub fn new(in_rate: u32, out_rate: u32) -> Self {
        Self {
            step: in_rate as f64 / out_rate as f64,
            frac: 0.0,
            hist: [[0.0; CH]; 4],
            primed: 0,
        }
    }

    pub fn nominal_step(in_rate: u32, out_rate: u32) -> f64 {
        in_rate as f64 / out_rate as f64
    }

    /// Fija la relación entrada/salida (incluida la corrección de deriva).
    pub fn set_step(&mut self, step: f64) {
        self.step = step;
    }

    /// Produce tramas de salida en `out` (entrelazado) tirando de `pull` cuando
    /// necesita una trama de entrada. `pull` devuelve `None` si no hay datos;
    /// en ese caso se detiene y devuelve cuántas tramas de salida ha escrito.
    pub fn process<F: FnMut() -> Option<[f32; CH]>>(&mut self, out: &mut [f32], mut pull: F) -> usize {
        let frames = out.len() / CH;
        // Cebado: la interpolación necesita x[-1..=2].
        while self.primed < 3 {
            match pull() {
                Some(f) => {
                    self.push(f);
                    self.primed += 1;
                }
                None => return 0,
            }
        }
        let mut written = 0;
        while written < frames {
            while self.frac >= 1.0 {
                match pull() {
                    Some(f) => {
                        self.push(f);
                        self.frac -= 1.0;
                    }
                    None => return written,
                }
            }
            let t = self.frac as f32;
            let [xm1, x0, x1, x2] = self.hist;
            for c in 0..CH {
                out[written * CH + c] = hermite(xm1[c], x0[c], x1[c], x2[c], t);
            }
            written += 1;
            self.frac += self.step;
        }
        written
    }

    #[inline]
    fn push(&mut self, f: [f32; CH]) {
        self.hist[0] = self.hist[1];
        self.hist[1] = self.hist[2];
        self.hist[2] = self.hist[3];
        self.hist[3] = f;
    }
}

#[inline]
fn hermite(xm1: f32, x0: f32, x1: f32, x2: f32, t: f32) -> f32 {
    let c0 = x0;
    let c1 = 0.5 * (x1 - xm1);
    let c2 = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
    let c3 = 0.5 * (x2 - xm1) + 1.5 * (x0 - x1);
    ((c3 * t + c2) * t + c1) * t + c0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn sine(rate: f32, freq: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| (2.0 * PI * freq * i as f32 / rate).sin()).collect()
    }

    /// Estima la frecuencia contando cruces por cero ascendentes.
    fn est_freq(x: &[f32], rate: f32) -> f32 {
        let mut cross = Vec::new();
        for i in 1..x.len() {
            if x[i - 1] < 0.0 && x[i] >= 0.0 {
                cross.push(i);
            }
        }
        let periods = (cross.len() - 1) as f32;
        let span = (cross[cross.len() - 1] - cross[0]) as f32;
        rate * periods / span
    }

    #[test]
    fn unity_ratio_is_identity_after_latency() {
        let input = sine(48000.0, 1000.0, 4800);
        let mut r = Resampler::<1>::new(48000, 48000);
        let mut it = input.iter().copied();
        let mut out = vec![0.0; 4000];
        let n = r.process(&mut out, || it.next().map(|s| [s]));
        assert_eq!(n, 4000);
        // Con relación 1,0 la salida es exactamente la entrada (sin retardo).
        for i in 0..4000 {
            assert!((out[i] - input[i]).abs() < 1e-6, "i={i}");
        }
    }

    #[test]
    fn converts_44100_to_48000_preserving_pitch() {
        let input = sine(44100.0, 1000.0, 44100);
        let mut r = Resampler::<1>::new(44100, 48000);
        let mut it = input.iter().copied();
        let mut out = vec![0.0; 47000];
        let n = r.process(&mut out, || it.next().map(|s| [s]));
        assert!(n > 46000);
        let f = est_freq(&out[..n], 48000.0);
        assert!((f - 1000.0).abs() < 1.0, "freq={f}");
        // Amplitud conservada (sin pérdidas graves de la interpolación a 1 kHz).
        let peak = out[1000..n].iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        assert!(peak > 0.98 && peak < 1.02, "peak={peak}");
    }

    #[test]
    fn stops_cleanly_on_underflow_and_resumes() {
        let mut r = Resampler::<2>::new(48000, 48000);
        let mut src: Vec<[f32; 2]> = (0..10).map(|i| [i as f32, -(i as f32)]).collect();
        src.reverse();
        let mut out = vec![0.0; 2 * 20];
        let n = r.process(&mut out, || src.pop());
        assert!(n < 20);
        let mut more: Vec<[f32; 2]> = (10..40).map(|i| [i as f32, -(i as f32)]).collect();
        more.reverse();
        let mut out2 = vec![0.0; 2 * 10];
        let m = r.process(&mut out2, || more.pop());
        assert_eq!(m, 10);
        // Continuidad: la rampa sigue sin saltos.
        assert!((out2[0] - (out[(n - 1) * 2] + 1.0)).abs() < 1e-4);
    }
}
