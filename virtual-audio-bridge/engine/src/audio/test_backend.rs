//! Backend SOLO PARA PRUEBAS AUTOMATIZADAS (feature `test-io`).
//!
//! Sustituye los dispositivos de Windows por:
//! * una fuente de tono de 440 Hz al ritmo del reloj del sistema (subida), y
//! * un consumidor al ritmo del reloj que mide el nivel recibido (bajada).
//!
//! No se compila en la versión de usuario. Sirve para comprobar en Linux/CI el
//! transporte WebRTC extremo a extremo con un navegador real (Chromium con
//! micrófono simulado). Los resultados obtenidos así son una SIMULACIÓN del
//! hardware de audio y se documentan como tal.

use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::{Control, DeviceEnds, DeviceInfo, DeviceStatus, RATE};
use crate::log::Log;
use crate::stats::{Stats, peak_10k};

pub fn start(ends: DeviceEnds, control: Arc<Control>, stats: Arc<Stats>, log: Log) -> (Vec<JoinHandle<()>>, DeviceStatus) {
    log.warn("Backend de PRUEBA activo (test-io): tono sintético y medidor, sin dispositivos reales");
    let DeviceEnds { mut up, mut down } = ends;
    let (c1, s1) = (control.clone(), stats.clone());
    let src = std::thread::spawn(move || {
        let t0 = Instant::now();
        let mut n: u64 = 0;
        let mut block = vec![0.0f32; 480 * 2];
        s1.capture_running.store(true, Relaxed);
        s1.capture_rate.store(RATE as u64, Relaxed);
        while !c1.stop.load(Relaxed) {
            let due = (t0.elapsed().as_secs_f64() * RATE as f64) as u64;
            while n + 480 <= due {
                for i in 0..480 {
                    let v = 0.5 * (2.0 * std::f64::consts::PI * 440.0 * (n + i as u64) as f64 / RATE as f64).sin();
                    block[2 * i] = v as f32;
                    block[2 * i + 1] = v as f32;
                }
                n += 480;
                s1.capture_frames.fetch_add(480, Relaxed);
                s1.tx_peak.fetch_max(peak_10k(&block), Relaxed);
                for &v in &block {
                    if up.push(v).is_err() {
                        s1.capture_overruns.fetch_add(1, Relaxed);
                        break;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    });
    let (c2, s2) = (control, stats);
    let sink = std::thread::spawn(move || {
        let t0 = Instant::now();
        let mut n: u64 = 0;
        let mut playing = false;
        let mut block = vec![0.0f32; 480];
        s2.render_running.store(true, Relaxed);
        s2.render_rate.store(RATE as u64, Relaxed);
        while !c2.stop.load(Relaxed) {
            let due = (t0.elapsed().as_secs_f64() * RATE as f64) as u64;
            while n + 480 <= due {
                n += 480;
                let target = c2.down_target_us.load(Relaxed) as usize * RATE as usize / 1_000_000;
                s2.rx_buffer_us.store(down.slots() as u64 * 1_000_000 / RATE as u64, Relaxed);
                s2.rx_target_us.store(c2.down_target_us.load(Relaxed), Relaxed);
                if !playing && down.slots() >= target {
                    playing = true;
                }
                if !playing {
                    continue;
                }
                let mut got = 0;
                while got < 480 {
                    match down.pop() {
                        Ok(v) => {
                            block[got] = v;
                            got += 1;
                        }
                        Err(_) => break,
                    }
                }
                if got < 480 {
                    s2.render_underruns.fetch_add(1, Relaxed);
                    playing = false;
                }
                s2.rx_peak.fetch_max(peak_10k(&block[..got]), Relaxed);
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    });
    let fake = |name: &str| DeviceInfo { id: name.into(), name: name.into(), state: "prueba".into(), is_default: false };
    (
        vec![src, sink],
        DeviceStatus {
            speaker: Some(fake("PRUEBA: tono 440 Hz")),
            mic_sink: Some(fake("PRUEBA: medidor")),
            ..Default::default()
        },
    )
}
