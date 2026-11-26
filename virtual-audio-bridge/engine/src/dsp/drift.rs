//! Control de deriva de reloj: mantiene el nivel del buffer de recepción en su
//! objetivo variando ligeramente la relación de remuestreo.
//!
//! El móvil produce audio con su reloj y el dispositivo de Windows lo consume
//! con el suyo; nunca coinciden exactamente (diferencias típicas de 10–200 ppm).
//! Sin corrección el buffer se vaciaría (cortes) o crecería sin límite (latencia).
//! Un controlador PI con límite de ±1000 ppm corrige la deriva de forma inaudible.

pub struct DriftController {
    kp: f64,
    ki: f64,
    integral: f64,
    /// Nivel filtrado (s) para no reaccionar a la llegada en ráfagas de paquetes.
    smoothed: f64,
    alpha: f64,
    limit: f64,
}

impl DriftController {
    /// `period_s`: tiempo entre llamadas a `update` (periodo del dispositivo).
    pub fn new(period_s: f64) -> Self {
        // Constante de tiempo de filtrado ~0,5 s; integración lenta.
        let alpha = (period_s / 0.5).min(1.0);
        Self {
            kp: 0.02,
            ki: 0.004 * period_s,
            integral: 0.0,
            smoothed: f64::NAN,
            alpha,
            limit: 1.0e-3,
        }
    }

    pub fn reset(&mut self) {
        self.integral = 0.0;
        self.smoothed = f64::NAN;
    }

    /// Devuelve la corrección relativa (p. ej. +0,0001 = consumir 100 ppm más rápido).
    pub fn update(&mut self, level_s: f64, target_s: f64) -> f64 {
        if self.smoothed.is_nan() {
            self.smoothed = level_s;
        } else {
            self.smoothed += self.alpha * (level_s - self.smoothed);
        }
        let err = self.smoothed - target_s; // >0: sobra audio → consumir más rápido
        self.integral = (self.integral + self.ki * err).clamp(-self.limit, self.limit);
        (self.kp * err + self.integral).clamp(-self.limit, self.limit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Simula un productor a 48 000·(1+d) Hz y un consumidor a 48 000 Hz con
    /// corrección: el nivel debe converger al objetivo.
    #[test]
    fn converges_with_clock_offset() {
        for &d in &[-300e-6, -50e-6, 0.0, 80e-6, 400e-6] {
            let period = 0.010;
            let mut c = DriftController::new(period);
            let target = 0.030;
            let mut level = 0.030;
            let mut corr = 0.0;
            for _ in 0..(600.0 / period) as usize {
                level += period * (1.0 + d); // producido
                level -= period * (1.0 + corr); // consumido
                corr = c.update(level, target);
            }
            assert!((level - target).abs() < 0.002, "d={d} level={level}");
            assert!((corr - d).abs() < 20e-6, "d={d} corr={corr}");
        }
    }

    #[test]
    fn correction_is_bounded() {
        let mut c = DriftController::new(0.01);
        for _ in 0..100000 {
            let r = c.update(1.0, 0.0);
            assert!(r <= 1.0e-3);
        }
    }
}
