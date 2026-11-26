//! Configuración persistente (JSON en el directorio de datos del usuario).

use std::path::PathBuf;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Config {
    /// IP local en la que escuchar. Vacío = detectar automáticamente.
    pub bind_ip: String,
    /// Puerto HTTPS (página del móvil y señalización).
    pub https_port: u16,
    /// Puerto UDP de WebRTC (ICE/DTLS/SRTP).
    pub udp_port: u16,
    /// Puerto de la interfaz local (solo 127.0.0.1).
    pub admin_port: u16,
    /// Subcadena del nombre del dispositivo de reproducción que se captura
    /// (el que eliges como salida en Windows). Vacío = el predeterminado.
    pub speaker_device: String,
    /// Subcadena del nombre del dispositivo donde se escribe el micrófono.
    /// Con VB-CABLE: "CABLE Input" (las aplicaciones graban de "CABLE Output").
    pub mic_sink_device: String,
    /// Bitrate de Opus hacia el móvil (bit/s).
    pub opus_bitrate: i32,
    /// Duración de trama Opus hacia el móvil (ms): 10 o 20.
    pub opus_frame_ms: u32,
    /// Límites del jitter buffer de bajada (ms).
    pub jitter_min_ms: u32,
    pub jitter_max_ms: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind_ip: String::new(),
            https_port: 47800,
            udp_port: 47801,
            admin_port: 47802,
            speaker_device: String::new(),
            mic_sink_device: "CABLE Input".into(),
            opus_bitrate: 128_000,
            opus_frame_ms: 10,
            jitter_min_ms: 15,
            jitter_max_ms: 200,
        }
    }
}

pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("VAB_DATA_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join("VirtualAudioBridge")))
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config/virtual-audio-bridge")))
        .unwrap_or_else(|| PathBuf::from("."));
    let _ = std::fs::create_dir_all(&base);
    base
}

impl Config {
    pub fn path() -> PathBuf {
        data_dir().join("config.json")
    }

    pub fn load() -> Result<Self, String> {
        match std::fs::read_to_string(Self::path()) {
            Ok(s) => serde_json::from_str::<Config>(&s)
                .map_err(|e| format!("config.json no válido: {e}"))
                .map(|c| c.sanitized()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("No se puede leer config.json: {e}")),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let s = serde_json::to_string_pretty(self).unwrap();
        std::fs::write(Self::path(), s).map_err(|e| format!("No se puede guardar config.json: {e}"))
    }

    pub fn sanitized(mut self) -> Self {
        if self.opus_frame_ms != 10 && self.opus_frame_ms != 20 {
            self.opus_frame_ms = 10;
        }
        self.opus_bitrate = self.opus_bitrate.clamp(16_000, 256_000);
        self.jitter_min_ms = self.jitter_min_ms.clamp(5, 100);
        self.jitter_max_ms = self.jitter_max_ms.clamp(self.jitter_min_ms + 10, 1000);
        self
    }
}
