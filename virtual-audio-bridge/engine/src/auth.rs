//! Autenticación local del móvil.
//!
//! * El código QR lleva un **token de emparejamiento** aleatorio de 128 bits, de
//!   un solo uso y con caducidad (10 min). Va en el fragmento `#` de la URL, que
//!   el navegador no envía en las peticiones ni guarda en registros del servidor.
//! * Al canjearlo, el servidor emite una **sesión** aleatoria de 256 bits en una
//!   cookie `HttpOnly; Secure; SameSite=Strict`. En memoria solo se guarda su hash.
//! * Tras demasiados intentos fallidos se bloquea el emparejamiento hasta generar
//!   un código nuevo desde el PC.
//! * No hay credenciales fijas: todo se genera en tiempo de ejecución.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const PAIR_TTL: Duration = Duration::from_secs(600);
const MAX_FAILS: u32 = 20;
const SESSION_TTL: Duration = Duration::from_secs(30 * 86400);

pub fn random_hex(bytes: usize) -> String {
    let mut b = vec![0u8; bytes];
    getrandom::fill(&mut b).expect("generador aleatorio del sistema");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn hash(s: &str) -> [u8; 32] {
    let d = ring::digest::digest(&ring::digest::SHA256, s.as_bytes());
    let mut o = [0u8; 32];
    o.copy_from_slice(d.as_ref());
    o
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub struct Auth {
    pair_token: Option<(String, Instant)>,
    fails: u32,
    /// hash(sesión) → creación (segundos Unix). Se guarda en disco (solo los
    /// hashes) para que reiniciar la aplicación no obligue a volver a emparejar.
    sessions: HashMap<[u8; 32], u64>,
    store: Option<PathBuf>,
}

fn now_s() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn hex_to_hash(h: &str) -> Option<[u8; 32]> {
    if h.len() != 64 {
        return None;
    }
    let mut o = [0u8; 32];
    for i in 0..32 {
        o[i] = u8::from_str_radix(&h[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(o)
}

#[derive(Debug, PartialEq)]
pub enum PairError {
    Invalid,
    Expired,
    Locked,
}

impl Default for Auth {
    fn default() -> Self {
        Self::new()
    }
}

impl Auth {
    pub fn new() -> Self {
        let mut a = Self { pair_token: None, fails: 0, sessions: HashMap::new(), store: None };
        a.new_pairing_token();
        a
    }

    /// Igual que `new`, pero carga y guarda las sesiones en `path`.
    pub fn with_store(path: PathBuf) -> Self {
        let mut a = Self::new();
        if let Ok(txt) = std::fs::read_to_string(&path) {
            for line in txt.lines() {
                let mut it = line.split_whitespace();
                if let (Some(h), Some(t)) = (it.next(), it.next()) {
                    if let (Some(h), Ok(t)) = (hex_to_hash(h), t.parse::<u64>()) {
                        a.sessions.insert(h, t);
                    }
                }
            }
        }
        a.store = Some(path);
        a.expire();
        a
    }

    fn save(&self) {
        let Some(p) = &self.store else { return };
        let mut out = String::new();
        for (h, t) in &self.sessions {
            out.push_str(&h.iter().map(|b| format!("{b:02x}")).collect::<String>());
            out.push_str(&format!(" {t}\n"));
        }
        let _ = std::fs::write(p, out);
    }

    fn expire(&mut self) {
        let now = now_s();
        let before = self.sessions.len();
        self.sessions.retain(|_, t| now.saturating_sub(*t) < SESSION_TTL.as_secs());
        if self.sessions.len() != before {
            self.save();
        }
    }

    /// Genera un nuevo código de emparejamiento (invalida el anterior).
    pub fn new_pairing_token(&mut self) -> String {
        let t = random_hex(16);
        self.pair_token = Some((t.clone(), Instant::now()));
        self.fails = 0;
        t
    }

    /// Token vigente para mostrar en el QR (lo renueva si caducó).
    pub fn current_pairing_token(&mut self) -> String {
        match &self.pair_token {
            Some((t, at)) if at.elapsed() < PAIR_TTL => t.clone(),
            _ => self.new_pairing_token(),
        }
    }

    pub fn pairing_expires_in(&self) -> Option<Duration> {
        self.pair_token.as_ref().map(|(_, at)| PAIR_TTL.saturating_sub(at.elapsed()))
    }

    /// Canjea el token por una sesión. El token queda consumido.
    pub fn pair(&mut self, token: &str) -> Result<String, PairError> {
        if self.fails >= MAX_FAILS {
            return Err(PairError::Locked);
        }
        let Some((t, at)) = &self.pair_token else {
            self.fails += 1;
            return Err(PairError::Invalid);
        };
        if !ct_eq(t.as_bytes(), token.as_bytes()) {
            self.fails += 1;
            return Err(PairError::Invalid);
        }
        if at.elapsed() >= PAIR_TTL {
            self.pair_token = None;
            return Err(PairError::Expired);
        }
        self.pair_token = None;
        let session = random_hex(32);
        self.sessions.insert(hash(&session), now_s());
        self.save();
        // Se prepara el siguiente código para emparejar otro dispositivo.
        self.new_pairing_token();
        Ok(session)
    }

    pub fn session_valid(&mut self, session: &str) -> bool {
        self.expire();
        let h = hash(session);
        self.sessions.keys().any(|k| ct_eq(k, &h))
    }

    /// Revoca todas las sesiones (el móvil tendrá que volver a escanear el QR).
    pub fn revoke_all(&mut self) {
        self.sessions.clear();
        self.save();
        self.new_pairing_token();
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_single_use() {
        let mut a = Auth::new();
        let t = a.current_pairing_token();
        let s = a.pair(&t).unwrap();
        assert!(a.session_valid(&s));
        assert_eq!(a.pair(&t), Err(PairError::Invalid));
        assert_ne!(a.current_pairing_token(), t);
    }

    #[test]
    fn wrong_token_locks_after_limit() {
        let mut a = Auth::new();
        for _ in 0..MAX_FAILS {
            assert_eq!(a.pair("nope"), Err(PairError::Invalid));
        }
        let t = a.current_pairing_token();
        assert_eq!(a.pair(&t), Err(PairError::Locked));
        let t2 = a.new_pairing_token();
        assert!(a.pair(&t2).is_ok());
    }

    #[test]
    fn revoke_invalidates_sessions() {
        let mut a = Auth::new();
        let t = a.current_pairing_token();
        let s = a.pair(&t).unwrap();
        a.revoke_all();
        assert!(!a.session_valid(&s));
        assert!(!a.session_valid("random"));
    }

    #[test]
    fn sessions_survive_restart() {
        let p = std::env::temp_dir().join(format!("vab-sessions-{}", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let mut a = Auth::with_store(p.clone());
        let t = a.current_pairing_token();
        let s = a.pair(&t).unwrap();
        drop(a);
        let mut b = Auth::with_store(p.clone());
        assert!(b.session_valid(&s));
        b.revoke_all();
        let mut c = Auth::with_store(p.clone());
        assert!(!c.session_valid(&s));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn tokens_are_random_and_long() {
        let a = random_hex(16);
        let b = random_hex(16);
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
    }
}
