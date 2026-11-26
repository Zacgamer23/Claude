//! Backend WASAPI (Windows).
//!
//! * Captura: *loopback* en modo compartido, guiado por eventos, del dispositivo
//!   de reproducción elegido como altavoz virtual. Además se abre en ese mismo
//!   dispositivo un flujo de reproducción que solo escribe silencio: sin él,
//!   WASAPI no entrega paquetes de loopback mientras nada suena, y el flujo hacia
//!   el móvil tendría huecos.
//! * Entrega: reproducción en modo compartido, guiada por eventos, en el
//!   dispositivo cuyo lado de grabación es el micrófono virtual (VB-CABLE:
//!   se escribe en "CABLE Input" y las aplicaciones graban de "CABLE Output").
//!
//! Los hilos se registran en MMCSS ("Pro Audio"). Dentro de los bucles no se
//! asigna memoria, no se bloquea y no se escribe en disco.

use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;
use std::thread::JoinHandle;
use std::time::Instant;

use wasapi::{
    BufferFlags, DeviceEnumerator, DeviceState, Direction, SampleType, StreamMode, WaveFormat,
};

use super::{Control, DeviceEnds, DeviceInfo, DeviceStatus, RATE, UP_CHANNELS, frame_to_stereo};
use crate::dsp::drift::DriftController;
use crate::dsp::resample::Resampler;
use crate::log::Log;
use crate::stats::{Stats, peak_10k};

type Res<T> = Result<T, String>;

fn e<E: std::fmt::Display>(ctx: &str) -> impl FnOnce(E) -> String + '_ {
    move |err| format!("{ctx}: {err}")
}

fn state_str(s: &DeviceState) -> String {
    match s {
        DeviceState::Active => "activo",
        DeviceState::Disabled => "deshabilitado",
        DeviceState::NotPresent => "no presente",
        DeviceState::Unplugged => "desconectado",
    }
    .to_string()
}

pub fn list_render_devices() -> Res<Vec<DeviceInfo>> {
    let _ = wasapi::initialize_mta();
    let en = DeviceEnumerator::new().map_err(e("DeviceEnumerator"))?;
    let default_id = en
        .get_default_device(Direction::Render)
        .ok()
        .and_then(|d| d.get_id().ok());
    let coll = en.get_device_collection(Direction::Render).map_err(e("enumerar dispositivos"))?;
    let mut out = Vec::new();
    for dev in &coll {
        let dev = dev.map_err(e("dispositivo"))?;
        let id = dev.get_id().unwrap_or_default();
        out.push(DeviceInfo {
            name: dev.get_friendlyname().unwrap_or_else(|_| "(sin nombre)".into()),
            state: dev.get_state().map(|s| state_str(&s)).unwrap_or_else(|_| "?".into()),
            is_default: Some(&id) == default_id.as_ref(),
            id,
        });
    }
    Ok(out)
}

fn find_render_device(en: &DeviceEnumerator, needle: &str) -> Res<wasapi::Device> {
    if needle.is_empty() {
        return en.get_default_device(Direction::Render).map_err(e("dispositivo predeterminado"));
    }
    let coll = en.get_device_collection(Direction::Render).map_err(e("enumerar dispositivos"))?;
    let lower = needle.to_lowercase();
    let mut names = Vec::new();
    for dev in &coll {
        let dev = dev.map_err(e("dispositivo"))?;
        let name = dev.get_friendlyname().unwrap_or_default();
        if name.to_lowercase().contains(&lower) {
            return Ok(dev);
        }
        names.push(name);
    }
    Err(format!(
        "No se encuentra ningún dispositivo de reproducción que contenga \"{needle}\". Disponibles: {}",
        names.join(" | ")
    ))
}

fn info(dev: &wasapi::Device) -> DeviceInfo {
    DeviceInfo {
        id: dev.get_id().unwrap_or_default(),
        name: dev.get_friendlyname().unwrap_or_default(),
        state: dev.get_state().map(|s| state_str(&s)).unwrap_or_else(|_| "?".into()),
        is_default: false,
    }
}

fn enable_mmcss() {
    use windows::Win32::System::Threading::AvSetMmThreadCharacteristicsW;
    use windows::core::w;
    let mut idx = 0u32;
    // Si falla, el hilo sigue con prioridad normal (se registra en el log del arranque).
    let _ = unsafe { AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut idx) };
}

pub fn start(
    speaker: &str,
    mic_sink: &str,
    ends: DeviceEnds,
    control: Arc<Control>,
    stats: Arc<Stats>,
    log: Log,
) -> (Vec<JoinHandle<()>>, DeviceStatus) {
    let mut status = DeviceStatus::default();
    let mut handles = Vec::new();
    let _ = wasapi::initialize_mta();
    let DeviceEnds { up, down } = ends;

    // Comprobación previa en este hilo para informar a la interfaz.
    match DeviceEnumerator::new() {
        Ok(en) => {
            match find_render_device(&en, speaker) {
                Ok(d) => status.speaker = Some(info(&d)),
                Err(m) => status.speaker_error = Some(m),
            }
            if mic_sink.is_empty() {
                status.mic_error = Some(
                    "No hay dispositivo configurado para el micrófono virtual (p. ej. \"CABLE Input\")".into(),
                );
            } else {
                match find_render_device(&en, mic_sink) {
                    Ok(d) => status.mic_sink = Some(info(&d)),
                    Err(m) => status.mic_error = Some(m),
                }
            }
        }
        Err(err) => {
            status.speaker_error = Some(format!("WASAPI no disponible: {err}"));
            status.mic_error = status.speaker_error.clone();
        }
    }
    if let (Some(s), Some(m)) = (&status.speaker, &status.mic_sink) {
        if s.id == m.id {
            status.mic_error = Some(
                "El altavoz y el micrófono virtual no pueden usar el mismo dispositivo: se crearía un bucle".into(),
            );
            status.mic_sink = None;
        }
    }

    if status.speaker.is_some() {
        let (c, s, l, name) = (control.clone(), stats.clone(), log.clone(), speaker.to_string());
        handles.push(
            std::thread::Builder::new()
                .name("vab-capture".into())
                .spawn(move || {
                    if let Err(m) = capture_thread(&name, up, &c, &s) {
                        l.error(&format!("Captura del altavoz virtual detenida: {m}"));
                    }
                    s.capture_running.store(false, Relaxed);
                })
                .expect("hilo de captura"),
        );
    }
    if status.mic_sink.is_some() {
        let (c, s, l, name) = (control, stats, log, mic_sink.to_string());
        handles.push(
            std::thread::Builder::new()
                .name("vab-render".into())
                .spawn(move || {
                    if let Err(m) = render_thread(&name, down, &c, &s) {
                        l.error(&format!("Entrega al micrófono virtual detenida: {m}"));
                    }
                    s.render_running.store(false, Relaxed);
                })
                .expect("hilo de entrega"),
        );
    }
    (handles, status)
}

/// Flujo de reproducción de silencio que mantiene vivo el loopback.
struct SilenceKeeper {
    client: wasapi::AudioClient,
    render: wasapi::AudioRenderClient,
    zeros: Vec<u8>,
    block_align: usize,
}

impl SilenceKeeper {
    fn new(dev: &wasapi::Device) -> Res<Self> {
        let mut client = dev.get_iaudioclient().map_err(e("cliente de silencio"))?;
        let fmt = client.get_mixformat().map_err(e("formato de mezcla"))?;
        let block_align = fmt.get_blockalign() as usize;
        let (def, _min) = client.get_device_period().map_err(e("periodo"))?;
        client
            .initialize_client(
                &fmt,
                Direction::Render,
                StreamMode::PollingShared { autoconvert: false, buffer_duration_hns: def * 4 },
                None,
            )
            .map_err(e("inicializar silencio"))?;
        let frames = client.get_buffer_size().map_err(e("buffer"))? as usize;
        let render = client.get_audiorenderclient().map_err(e("render de silencio"))?;
        client.start_stream().map_err(e("arrancar silencio"))?;
        Ok(Self { client, render, zeros: vec![0u8; frames * block_align], block_align })
    }

    fn feed(&mut self) {
        if let Ok(n) = self.client.get_available_space_in_frames() {
            let bytes = n as usize * self.block_align;
            if bytes > 0 && bytes <= self.zeros.len() {
                let _ = self
                    .render
                    .write_to_device(&self.zeros[..bytes], Some(BufferFlags::new(0x2 /* SILENT */)));
            }
        }
    }
}

fn capture_thread(name: &str, mut up: rtrb::Producer<f32>, control: &Control, stats: &Stats) -> Res<()> {
    let _ = wasapi::initialize_mta();
    enable_mmcss();
    let en = DeviceEnumerator::new().map_err(e("DeviceEnumerator"))?;
    let dev = find_render_device(&en, name)?;
    let mut client = dev.get_iaudioclient().map_err(e("cliente de loopback"))?;
    let mix = client.get_mixformat().map_err(e("formato de mezcla"))?;
    let rate = mix.get_samplespersec();
    let ch = mix.get_nchannels() as usize;
    // f32 con la frecuencia y canales del motor: sin conversión de frecuencia en Windows.
    let fmt = WaveFormat::new(32, 32, SampleType::Float, rate as usize, ch, None);
    let (_def, min) = client.get_device_period().map_err(e("periodo"))?;
    client
        .initialize_client(
            &fmt,
            Direction::Capture, // Capture sobre un dispositivo Render = loopback
            StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: min },
            None,
        )
        .map_err(e("inicializar loopback"))?;
    let event = client.set_get_eventhandle().map_err(e("evento"))?;
    let cap = client.get_audiocaptureclient().map_err(e("captura"))?;
    let buf_frames = client.get_buffer_size().map_err(e("buffer"))? as usize;
    let mut raw = vec![0u8; buf_frames * ch * 4 * 4];
    let mut keeper = SilenceKeeper::new(&dev).ok();

    let mut rs = Resampler::<2>::new(rate, RATE);
    let mut out = vec![0.0f32; (buf_frames * 4 * RATE as usize / rate as usize + 16) * UP_CHANNELS];

    stats.capture_rate.store(rate as u64, Relaxed);
    stats.capture_period_us.store((min / 10) as u64, Relaxed);
    stats.capture_running.store(true, Relaxed);
    client.start_stream().map_err(e("arrancar loopback"))?;

    while !control.stop.load(Relaxed) {
        if let Some(k) = keeper.as_mut() {
            k.feed();
        }
        if event.wait_for_event(100).is_err() {
            continue; // sin datos durante 100 ms (no debería ocurrir con el keeper)
        }
        loop {
            let Some(n) = cap.get_next_packet_size().map_err(e("paquete"))? else { break };
            if n == 0 {
                break;
            }
            let need = n as usize * ch * 4;
            if need > raw.len() {
                return Err("paquete de loopback mayor que el buffer".into());
            }
            let (frames, info) = cap.read_from_device(&mut raw[..need]).map_err(e("leer loopback"))?;
            let frames = frames as usize;
            if info.flags.data_discontinuity {
                stats.capture_overruns.fetch_add(1, Relaxed);
            }
            stats.capture_frames.fetch_add(frames as u64, Relaxed);
            if !control.bridge_enabled.load(Relaxed) {
                continue;
            }
            // Conversión bytes→f32 y remuestreo (identidad si rate == 48 kHz).
            let samples: &[f32] = unsafe {
                std::slice::from_raw_parts(raw.as_ptr() as *const f32, frames * ch)
            };
            let mut i = 0usize;
            let produced = rs.process(&mut out, || {
                if i < frames {
                    let f = frame_to_stereo(&samples[i * ch..i * ch + ch]);
                    i += 1;
                    Some(f)
                } else {
                    None
                }
            });
            let block = &out[..produced * UP_CHANNELS];
            stats.tx_peak.fetch_max(peak_10k(block), Relaxed);
            push_all(&mut up, block, &stats.capture_overruns);
        }
    }
    let _ = client.stop_stream();
    Ok(())
}

/// Escribe en la cola; si no cabe, descarta lo que no cabe y lo cuenta.
fn push_all(p: &mut rtrb::Producer<f32>, data: &[f32], overruns: &std::sync::atomic::AtomicU64) {
    let n = data.len().min(p.slots());
    if n < data.len() {
        overruns.fetch_add(1, Relaxed);
    }
    if n == 0 {
        return;
    }
    if let Ok(mut chunk) = p.write_chunk_uninit(n) {
        let (a, b) = chunk.as_mut_slices();
        for (dst, src) in a.iter_mut().chain(b.iter_mut()).zip(&data[..n]) {
            dst.write(*src);
        }
        unsafe { chunk.commit_all() };
    }
}

fn render_thread(name: &str, mut down: rtrb::Consumer<f32>, control: &Control, stats: &Stats) -> Res<()> {
    let _ = wasapi::initialize_mta();
    enable_mmcss();
    let en = DeviceEnumerator::new().map_err(e("DeviceEnumerator"))?;
    let dev = find_render_device(&en, name)?;
    let mut client = dev.get_iaudioclient().map_err(e("cliente de reproducción"))?;
    let mix = client.get_mixformat().map_err(e("formato de mezcla"))?;
    let rate = mix.get_samplespersec();
    let ch = mix.get_nchannels() as usize;
    let fmt = WaveFormat::new(32, 32, SampleType::Float, rate as usize, ch, None);
    let (_def, min) = client.get_device_period().map_err(e("periodo"))?;
    client
        .initialize_client(
            &fmt,
            Direction::Render,
            StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: min },
            None,
        )
        .map_err(e("inicializar reproducción"))?;
    let event = client.set_get_eventhandle().map_err(e("evento"))?;
    let render = client.get_audiorenderclient().map_err(e("render"))?;
    let buf_frames = client.get_buffer_size().map_err(e("buffer"))? as usize;
    let period_s = min as f64 / 1.0e7;

    let mut mono = vec![0.0f32; buf_frames];
    let mut bytes = vec![0u8; buf_frames * ch * 4];
    let nominal = Resampler::<1>::nominal_step(RATE, rate);
    let mut rs = Resampler::<1>::new(RATE, rate);
    let mut drift = DriftController::new(period_s);
    let mut playing = false;
    let started = Instant::now();
    let _ = started;

    stats.render_rate.store(rate as u64, Relaxed);
    stats.render_period_us.store((min / 10) as u64, Relaxed);
    stats.render_running.store(true, Relaxed);

    // Pre-relleno de silencio y arranque.
    render
        .write_to_device(&bytes[..buf_frames * ch * 4], Some(BufferFlags::new(0x2)))
        .map_err(e("pre-relleno"))?;
    client.start_stream().map_err(e("arrancar reproducción"))?;

    while !control.stop.load(Relaxed) {
        if event.wait_for_event(100).is_err() {
            continue;
        }
        let avail = client.get_available_space_in_frames().map_err(e("espacio"))? as usize;
        if avail == 0 {
            continue;
        }
        let avail = avail.min(buf_frames);
        let level_s = down.slots() as f64 / RATE as f64;
        let target_s = control.down_target_us.load(Relaxed) as f64 / 1.0e6;
        stats.rx_buffer_us.store((level_s * 1.0e6) as u64, Relaxed);
        stats.rx_target_us.store((target_s * 1.0e6) as u64, Relaxed);

        // Desbordamiento: más de objetivo + 150 ms → descartar el exceso de golpe.
        if level_s > target_s + 0.150 {
            let drop = ((level_s - target_s) * RATE as f64) as usize;
            if let Ok(c) = down.read_chunk(drop.min(down.slots())) {
                c.commit_all();
            }
            stats.render_overruns.fetch_add(1, Relaxed);
            drift.reset();
        }

        let mut written = 0usize;
        if !control.bridge_enabled.load(Relaxed) {
            playing = false;
        } else if !playing {
            // Pre-buffer: no se empieza hasta alcanzar el objetivo.
            if level_s >= target_s {
                playing = true;
                drift.reset();
            }
        }
        if playing {
            let corr = drift.update(down.slots() as f64 / RATE as f64, target_s);
            rs.set_step(nominal * (1.0 + corr));
            stats.drift_ppm.store((corr * 1.0e6) as i64, Relaxed);
            written = rs.process(&mut mono[..avail], || down.pop().ok().map(|s| [s]));
            if written < avail {
                stats.render_underruns.fetch_add(1, Relaxed);
                playing = false; // volver a pre-buffer para no encadenar cortes
            }
            stats.rx_peak.fetch_max(peak_10k(&mono[..written]), Relaxed);
        }
        for v in &mut mono[written..avail] {
            *v = 0.0;
        }
        // Mono → todos los canales, f32 LE.
        let out = &mut bytes[..avail * ch * 4];
        for (i, s) in mono[..avail].iter().enumerate() {
            let b = s.to_le_bytes();
            for c in 0..ch {
                let o = (i * ch + c) * 4;
                out[o..o + 4].copy_from_slice(&b);
            }
        }
        render.write_to_device(out, None).map_err(e("escribir"))?;
    }
    let _ = client.stop_stream();
    Ok(())
}
