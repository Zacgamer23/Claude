//! Estado compartido de la aplicación y servidores HTTP.

pub mod admin;
pub mod phone;

use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

use crate::audio::{self, AudioThreads, Control, DeviceStatus};
use crate::auth::Auth;
use crate::config::Config;
use crate::log::Log;
use crate::rtc::RtcCmd;
use crate::stats::Stats;
use crate::tls::TlsMaterial;

#[derive(Default, Clone, serde::Serialize)]
pub struct PhoneState {
    pub ws_connected: bool,
    /// Último informe de la página del móvil (estadísticas del navegador,
    /// etiqueta de la entrada de audio, visibilidad, modo).
    pub report: serde_json::Value,
    pub user_agent: String,
}

pub struct App {
    pub auth: Mutex<Auth>,
    pub rtc_tx: std::sync::mpsc::Sender<RtcCmd>,
    pub stats: Arc<Stats>,
    pub control: Arc<Control>,
    pub log: Log,
    pub config: Mutex<Config>,
    pub tls: TlsMaterial,
    /// `https://IP:puerto`
    pub origin: String,
    pub admin_token: String,
    pub phone: Mutex<PhoneState>,
    /// Se incrementa para cerrar la conexión WebSocket activa del móvil.
    pub ws_epoch: AtomicU64,
    /// Motivo del último cierre forzado desde el PC ("pc" o "revoked").
    pub kick_reason: Mutex<&'static str>,
    pub devices: Mutex<DeviceStatus>,
    pub audio: Mutex<Option<AudioThreads>>,
    pub started: std::time::Instant,
    /// Señal para cerrar la aplicación desde la interfaz.
    pub quit: tokio::sync::Notify,
}

impl App {
    pub fn pairing_url(&self) -> String {
        let t = self.auth.lock().unwrap().current_pairing_token();
        format!("{}/#t={}", self.origin, t)
    }

    /// (Re)arranca los hilos de los dispositivos de audio con la configuración actual.
    pub fn restart_audio(&self) {
        let (speaker, mic) = {
            let c = self.config.lock().unwrap();
            (c.speaker_device.clone(), c.mic_sink_device.clone())
        };
        if let Some(t) = self.audio.lock().unwrap().take() {
            t.stop();
        }
        let (net, dev) = audio::make_pipes();
        let _ = self.rtc_tx.send(RtcCmd::SetPipes(net));
        let (threads, status) =
            audio::start(&speaker, &mic, dev, self.control.clone(), self.stats.clone(), self.log.clone());
        if let Some(s) = &status.speaker {
            self.log.info(&format!("Altavoz virtual (captura): {}", s.name));
        }
        if let Some(e) = &status.speaker_error {
            self.log.error(&format!("Altavoz virtual: {e}"));
        }
        if let Some(m) = &status.mic_sink {
            self.log.info(&format!("Micrófono virtual (entrega en): {}", m.name));
        }
        if let Some(e) = &status.mic_error {
            self.log.error(&format!("Micrófono virtual: {e}"));
        }
        *self.devices.lock().unwrap() = status;
        *self.audio.lock().unwrap() = Some(threads);
    }
}

pub fn json_err(code: axum::http::StatusCode, msg: &str) -> axum::response::Response {
    use axum::response::IntoResponse;
    (code, axum::Json(serde_json::json!({ "ok": false, "error": msg }))).into_response()
}

/// Cabeceras de seguridad comunes de las páginas.
pub fn secure_headers(mut r: axum::response::Response, csp_connect: &str) -> axum::response::Response {
    use axum::http::HeaderValue;
    let h = r.headers_mut();
    h.insert("Cache-Control", HeaderValue::from_static("no-store"));
    h.insert("X-Content-Type-Options", HeaderValue::from_static("nosniff"));
    h.insert("Referrer-Policy", HeaderValue::from_static("no-referrer"));
    h.insert("X-Frame-Options", HeaderValue::from_static("DENY"));
    if let Ok(v) = HeaderValue::from_str(&format!(
        "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self' {csp_connect}; media-src 'self' blob: mediastream:; frame-ancestors 'none'; base-uri 'none'; form-action 'none'"
    )) {
        h.insert("Content-Security-Policy", v);
    }
    r
}
