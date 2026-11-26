//! Transporte WebRTC con el navegador del móvil (str0m, *sans-IO*).
//!
//! * ICE-lite en el PC: el navegador hace las comprobaciones contra el
//!   candidato *host* del PC (su IP local). No hace falta STUN/TURN ni resolver
//!   los candidatos mDNS del navegador: el PC aprende su dirección como
//!   candidato *peer-reflexive*. Todo el tráfico queda en la red local.
//! * Un único socket UDP en un puerto fijo (para la regla del firewall).
//! * Subida: Opus estéreo, tramas de 10 ms, FEC en banda, `LowDelay`.
//! * Bajada: Opus → PLC/FEC → cola hacia el hilo del micrófono virtual, con un
//!   objetivo de jitter buffer adaptativo medido en este hilo.

use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use str0m::change::{SdpAnswer, SdpOffer};
use str0m::format::Codec;
use str0m::media::{Frequency, MediaKind, MediaTime, Mid, Pt};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc};

use crate::audio::{Control, NetEnds, RATE, UP_CHANNELS};
use crate::dsp::jitter::JitterEstimator;
use crate::log::Log;
use crate::stats::{Stats, peak_10k};

pub enum RtcCmd {
    Offer { offer: SdpOffer, reply: tokio::sync::oneshot::Sender<Result<SdpAnswer, String>> },
    Disconnect,
    SetPipes(NetEnds),
    Shutdown,
}

#[derive(Clone, Copy)]
pub struct RtcParams {
    pub bitrate: i32,
    pub frame_ms: u32,
    pub jitter_min_ms: u32,
    pub jitter_max_ms: u32,
}

struct Peer {
    rtc: Rtc,
    mid: Option<Mid>,
    pt: Option<Pt>,
    connected: bool,
    since: Instant,
}

/// Periodo de referencia del dispositivo de salida para el objetivo del buffer.
const OUT_PERIOD_S: f64 = 0.010;
/// Latencia máxima admitida en la cola de subida antes de descartar (ms).
const UP_MAX_QUEUE_MS: usize = 30;

pub fn spawn(
    socket: UdpSocket,
    rx: Receiver<RtcCmd>,
    ends: NetEnds,
    control: Arc<Control>,
    stats: Arc<Stats>,
    log: Log,
    params: RtcParams,
) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("vab-rtc".into())
        .spawn(move || {
            let mut w = Worker::new(socket, ends, control, stats, log.clone(), params);
            if let Err(e) = w.run(rx) {
                log.error(&format!("Hilo WebRTC detenido: {e}"));
            }
        })
        .expect("hilo WebRTC")
}

struct Worker {
    socket: UdpSocket,
    local: SocketAddr,
    ends: NetEnds,
    control: Arc<Control>,
    stats: Arc<Stats>,
    log: Log,
    params: RtcParams,
    peer: Option<Peer>,
    enc: opus::Encoder,
    dec: opus::Decoder,
    frame: usize,
    up_buf: Vec<f32>,
    enc_out: Vec<u8>,
    pcm: Vec<f32>,
    up_ts: u64,
    next_rx_ts: Option<u64>,
    jitter: JitterEstimator,
    t0: Instant,
}

impl Worker {
    fn new(socket: UdpSocket, ends: NetEnds, control: Arc<Control>, stats: Arc<Stats>, log: Log, params: RtcParams) -> Self {
        let local = socket.local_addr().expect("dirección local del socket UDP");
        let frame = (RATE as usize * params.frame_ms as usize) / 1000;
        Self {
            socket,
            local,
            ends,
            control,
            stats,
            log,
            params,
            peer: None,
            enc: make_encoder(params),
            dec: opus::Decoder::new(RATE, opus::Channels::Mono).expect("decodificador Opus"),
            frame,
            up_buf: vec![0.0; frame * UP_CHANNELS],
            enc_out: vec![0u8; 4000],
            pcm: vec![0.0; 5760],
            up_ts: 0,
            next_rx_ts: None,
            jitter: JitterEstimator::new(250, params.jitter_min_ms as f64 / 1000.0, params.jitter_max_ms as f64 / 1000.0),
            t0: Instant::now(),
        }
    }

    fn run(&mut self, rx: Receiver<RtcCmd>) -> Result<(), String> {
        let mut buf = vec![0u8; 2000];
        loop {
            // 1. Órdenes de la capa web.
            loop {
                match rx.try_recv() {
                    Ok(RtcCmd::Offer { offer, reply }) => {
                        let _ = reply.send(self.accept_offer(offer));
                    }
                    Ok(RtcCmd::Disconnect) => self.drop_peer("desconectado desde el PC"),
                    Ok(RtcCmd::SetPipes(ends)) => self.ends = ends,
                    Ok(RtcCmd::Shutdown) | Err(TryRecvError::Disconnected) => {
                        self.drop_peer("cierre de la aplicación");
                        return Ok(());
                    }
                    Err(TryRecvError::Empty) => break,
                }
            }

            // 2. Salida de str0m (paquetes a enviar, eventos) hasta su próximo timeout.
            let mut deadline = Instant::now() + Duration::from_millis(50);
            if let Some(t) = self.poll_peer()? {
                deadline = deadline.min(t);
            }

            // 3. Audio del PC hacia el móvil.
            self.send_up();

            // 4. Esperar red. Con conexión activa se limita a 2 ms para que el
            //    audio capturado no espere en la cola (latencia de subida).
            let max_wait = if self.peer.as_ref().map(|p| p.connected).unwrap_or(false) {
                Duration::from_millis(2)
            } else {
                Duration::from_millis(50)
            };
            let wait = deadline.saturating_duration_since(Instant::now()).clamp(Duration::from_millis(1), max_wait);
            self.socket.set_read_timeout(Some(wait)).map_err(|e| e.to_string())?;
            match self.socket.recv_from(&mut buf) {
                Ok((n, source)) => {
                    if let Some(peer) = self.peer.as_mut() {
                        if let Ok(contents) = buf[..n].try_into() {
                            let input = Input::Receive(
                                Instant::now(),
                                Receive { proto: Protocol::Udp, source, destination: self.local, contents },
                            );
                            if peer.rtc.accepts(&input) {
                                if let Err(e) = peer.rtc.handle_input(input) {
                                    self.drop_peer(&format!("error de WebRTC: {e}"));
                                }
                            }
                        }
                    }
                }
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {
                    // Windows devuelve WSAECONNRESET tras un ICMP "puerto inalcanzable"; se ignora.
                }
                Err(e) => return Err(format!("socket UDP: {e}")),
            }
            if let Some(peer) = self.peer.as_mut() {
                if let Err(e) = peer.rtc.handle_input(Input::Timeout(Instant::now())) {
                    self.drop_peer(&format!("error de WebRTC: {e}"));
                }
            }
        }
    }

    fn accept_offer(&mut self, offer: SdpOffer) -> Result<SdpAnswer, String> {
        self.drop_peer("nueva conexión del móvil");
        let mut rtc = Rtc::builder()
            .set_ice_lite(true)
            .clear_codecs()
            .enable_opus(true, false)
            // str0m reordena por sí mismo; aquí solo se espera muy poco porque el
            // jitter buffer propio absorbe el resto y los huecos se ocultan con PLC.
            .set_reordering_size_audio(2)
            .set_reordering_timeout_audio(Some(Duration::from_millis(20)))
            .set_stats_interval(Some(Duration::from_secs(1)))
            .build(Instant::now());
        let cand = Candidate::host(self.local, "udp").map_err(|e| format!("candidato: {e}"))?;
        rtc.add_local_candidate(cand);
        let answer = rtc.sdp_api().accept_offer(offer).map_err(|e| format!("oferta SDP rechazada: {e}"))?;
        self.peer = Some(Peer { rtc, mid: None, pt: None, connected: false, since: Instant::now() });
        self.next_rx_ts = None;
        self.jitter.reset();
        self.dec = opus::Decoder::new(RATE, opus::Channels::Mono).map_err(|e| e.to_string())?;
        self.enc = make_encoder(self.params);
        self.log.info("Oferta WebRTC aceptada; esperando conexión ICE/DTLS");
        Ok(answer)
    }

    fn drop_peer(&mut self, why: &str) {
        if let Some(mut p) = self.peer.take() {
            p.rtc.disconnect();
            self.log.info(&format!("Conexión WebRTC cerrada: {why}"));
        }
        self.stats.peer_connected.store(false, Relaxed);
    }

    /// Procesa la salida de str0m. Devuelve el próximo instante de timeout.
    fn poll_peer(&mut self) -> Result<Option<Instant>, String> {
        loop {
            let Some(peer) = self.peer.as_mut() else { return Ok(None) };
            if !peer.rtc.is_alive() {
                self.drop_peer("la sesión WebRTC terminó");
                return Ok(None);
            }
            // Si ICE no conecta en 20 s, se informa (aislamiento Wi-Fi / firewall).
            if !peer.connected && peer.since.elapsed() > Duration::from_secs(20) {
                self.log.error(
                    "El móvil no ha podido conectar por UDP en 20 s. Causas habituales: el Firewall de Windows \
                     bloquea el puerto UDP, la red es \"Pública\" en Windows, o el router aísla a los clientes Wi-Fi.",
                );
                self.drop_peer("tiempo de conexión agotado");
                return Ok(None);
            }
            let out = match peer.rtc.poll_output() {
                Ok(o) => o,
                Err(e) => {
                    self.drop_peer(&format!("error de WebRTC: {e}"));
                    return Ok(None);
                }
            };
            match out {
                Output::Timeout(t) => return Ok(Some(t)),
                Output::Transmit(t) => {
                    if let Err(e) = self.socket.send_to(&t.contents, t.destination) {
                        if e.kind() != std::io::ErrorKind::WouldBlock {
                            self.log.warn(&format!("envío UDP: {e}"));
                        }
                    }
                }
                Output::Event(ev) => self.on_event(ev),
            }
        }
    }

    fn on_event(&mut self, ev: Event) {
        match ev {
            Event::Connected => {
                if let Some(p) = self.peer.as_mut() {
                    p.connected = true;
                }
                self.stats.peer_connected.store(true, Relaxed);
                self.log.info("Móvil conectado (ICE + DTLS-SRTP establecidos)");
            }
            Event::IceConnectionStateChange(s) => {
                if s == IceConnectionState::Disconnected {
                    self.drop_peer("ICE desconectado (el móvil dejó de responder)");
                }
            }
            Event::MediaAdded(m) => {
                if m.kind == MediaKind::Audio {
                    if let Some(p) = self.peer.as_mut() {
                        p.mid = Some(m.mid);
                    }
                }
            }
            Event::MediaData(d) => self.on_rx(d),
            Event::PeerStats(s) => {
                if let Some(rtt) = s.rtt {
                    self.stats.rtt_us.store(rtt.as_micros() as u64, Relaxed);
                }
                if let Some(l) = s.ingress_loss_fraction {
                    self.stats.ingress_loss_permille.store((l * 1000.0) as u64, Relaxed);
                }
                if let Some(l) = s.egress_loss_fraction {
                    self.stats.egress_loss_permille.store((l * 1000.0) as u64, Relaxed);
                }
            }
            Event::MediaIngressStats(s) => {
                if let Some(rtt) = s.rtt {
                    self.stats.rtt_us.store(rtt.as_micros() as u64, Relaxed);
                }
            }
            _ => {}
        }
    }

    fn send_up(&mut self) {
        let frame_samples = self.frame * UP_CHANNELS;
        let enabled = self.control.bridge_enabled.load(Relaxed);
        let ready = self
            .peer
            .as_ref()
            .map(|p| p.connected && p.mid.is_some())
            .unwrap_or(false);
        let up = &mut self.ends.up;
        if !ready || !enabled {
            // Sin conexión: se descarta para no acumular latencia.
            let n = up.slots();
            if let Ok(c) = up.read_chunk(n) {
                c.commit_all();
            }
            return;
        }
        // Límite de cola: si se acumuló (p. ej. tras una pausa), se descarta lo más antiguo.
        let max = UP_MAX_QUEUE_MS * RATE as usize / 1000 * UP_CHANNELS;
        if up.slots() > max {
            let drop = (up.slots() - frame_samples) / frame_samples * frame_samples;
            if let Ok(c) = up.read_chunk(drop) {
                c.commit_all();
            }
            self.stats.capture_overruns.fetch_add(1, Relaxed);
        }
        while up.slots() >= frame_samples {
            let Ok(chunk) = up.read_chunk(frame_samples) else { break };
            let (a, b) = chunk.as_slices();
            self.up_buf[..a.len()].copy_from_slice(a);
            self.up_buf[a.len()..a.len() + b.len()].copy_from_slice(b);
            chunk.commit_all();

            let len = match self.enc.encode_float(&self.up_buf, &mut self.enc_out) {
                Ok(l) => l,
                Err(e) => {
                    self.log.warn(&format!("Opus encode: {e}"));
                    continue;
                }
            };
            let Some(peer) = self.peer.as_mut() else { return };
            let mid = peer.mid.unwrap();
            let Some(writer) = peer.rtc.writer(mid) else { return };
            let pt = match peer.pt {
                Some(pt) => pt,
                None => {
                    let Some(p) = writer.payload_params().find(|p| p.spec().codec == Codec::Opus) else {
                        self.log.error("El navegador no ha negociado Opus");
                        return;
                    };
                    peer.pt = Some(p.pt());
                    p.pt()
                }
            };
            let t = MediaTime::new(self.up_ts, Frequency::FORTY_EIGHT_KHZ);
            self.up_ts += self.frame as u64;
            if let Err(e) = writer.write(pt, Instant::now(), t, self.enc_out[..len].to_vec()) {
                self.log.warn(&format!("envío RTP: {e}"));
                return;
            }
            self.stats.tx_packets.fetch_add(1, Relaxed);
            self.stats.tx_bytes.fetch_add(len as u64, Relaxed);
        }
    }

    fn on_rx(&mut self, d: str0m::media::MediaData) {
        self.stats.rx_packets.fetch_add(1, Relaxed);
        self.stats.rx_bytes.fetch_add(d.data.len() as u64, Relaxed);
        let ts = d.time.rebase(Frequency::FORTY_EIGHT_KHZ).numer();
        let arrival = d.network_time.saturating_duration_since(self.t0).as_secs_f64();
        self.jitter.on_packet(arrival, ts as f64 / RATE as f64);
        let nb = opus::packet::get_nb_samples(&d.data, RATE).unwrap_or(960);

        if let Some(exp) = self.next_rx_ts {
            if ts < exp {
                return; // duplicado o demasiado tarde: ya se ocultó con PLC
            }
            let gap = (ts - exp) as usize;
            if gap > 0 && gap <= RATE as usize * 120 / 1000 {
                if gap == nb {
                    // Un único paquete perdido: se reconstruye con el FEC del actual.
                    match self.dec.decode_float(&d.data, &mut self.pcm[..gap], true) {
                        Ok(n) => self.push_down(n),
                        Err(_) => self.plc(gap),
                    }
                } else {
                    self.plc(gap);
                }
                self.stats.rx_plc_frames.fetch_add(gap as u64, Relaxed);
            } else if gap > 0 {
                // Hueco grande (pausa del móvil): se reinicia la referencia.
                self.jitter.reset();
            }
        }
        match self.dec.decode_float(&d.data, &mut self.pcm, false) {
            Ok(n) => {
                self.push_down(n);
                self.next_rx_ts = Some(ts + n as u64);
            }
            Err(_) => {
                self.stats.rx_decode_errors.fetch_add(1, Relaxed);
                self.next_rx_ts = Some(ts + nb as u64);
            }
        }
        let tgt = self.jitter.target_s(nb as f64 / RATE as f64, OUT_PERIOD_S);
        self.control.down_target_us.store((tgt * 1.0e6) as u64, Relaxed);
        self.stats.rx_jitter_us.store((self.jitter.jitter_s() * 1.0e6) as u64, Relaxed);
    }

    fn plc(&mut self, mut samples: usize) {
        while samples > 0 {
            let n = samples.min(self.pcm.len());
            match self.dec.decode_float(&[], &mut self.pcm[..n], false) {
                Ok(m) if m > 0 => {
                    self.push_down(m);
                    samples = samples.saturating_sub(m);
                }
                _ => break,
            }
        }
    }

    fn push_down(&mut self, n: usize) {
        if !self.control.bridge_enabled.load(Relaxed) {
            return;
        }
        let data = &self.pcm[..n];
        self.stats.rx_peak.fetch_max(peak_10k(data), Relaxed);
        let p = &mut self.ends.down;
        let m = n.min(p.slots());
        if m < n {
            self.stats.render_overruns.fetch_add(1, Relaxed);
        }
        if let Ok(mut chunk) = p.write_chunk_uninit(m) {
            let (a, b) = chunk.as_mut_slices();
            for (dst, src) in a.iter_mut().chain(b.iter_mut()).zip(data) {
                dst.write(*src);
            }
            unsafe { chunk.commit_all() };
        }
    }
}

fn make_encoder(p: RtcParams) -> opus::Encoder {
    let mut enc = opus::Encoder::new(RATE, opus::Channels::Stereo, opus::Application::LowDelay).expect("codificador Opus");
    let _ = enc.set_bitrate(opus::Bitrate::Bits(p.bitrate));
    let _ = enc.set_inband_fec(true);
    let _ = enc.set_packet_loss_perc(5);
    enc
}
