//! HTTPS en red local sin servicios externos.
//!
//! Se genera una **CA local propia** (la clave privada nunca sale del PC) y, con
//! ella, el certificado del servidor con la IP local en el SAN.
//!
//! Dos formas de usarlo en el iPhone, sin desactivar ninguna medida de seguridad:
//! 1. Aceptar el aviso de Safari una vez ("Mostrar detalles → visitar este sitio web").
//! 2. (Recomendado) Instalar el certificado de la CA como perfil en iOS y activar
//!    la confianza total en Ajustes → General → Información → Ajustes de confianza
//!    de certificados. Después no hay avisos y WSS funciona sin excepciones.
//!
//! Requisitos de Apple para certificados TLS que se cumplen aquí: SAN con la IP,
//! EKU serverAuth, clave ECDSA P-256, validez del certificado del servidor ≤ 825
//! días (se usan 397).

use std::net::{IpAddr, Ipv4Addr};
use std::path::Path;

use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer,
    KeyPair, KeyUsagePurpose, SanType,
};
use time::{Duration, OffsetDateTime};

pub struct TlsMaterial {
    pub cert_pem: String,
    pub key_pem: String,
    pub ca_der: Vec<u8>,
    /// Huella SHA-256 del certificado de la CA (para comprobar el perfil en el móvil).
    pub ca_fingerprint: String,
    /// Huella SHA-256 del certificado del servidor.
    pub cert_fingerprint: String,
}

const CA_NAME: &str = "Virtual Audio Bridge Local CA";

fn ca_params() -> CertificateParams {
    let mut p = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, CA_NAME);
    dn.push(DnType::OrganizationName, "Virtual Audio Bridge (local)");
    p.distinguished_name = dn;
    p.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    p.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign, KeyUsagePurpose::DigitalSignature];
    p.not_before = OffsetDateTime::now_utc() - Duration::days(1);
    p.not_after = OffsetDateTime::now_utc() + Duration::days(3650);
    p
}

pub fn fingerprint(der: &[u8]) -> String {
    let d = ring::digest::digest(&ring::digest::SHA256, der);
    d.as_ref().iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":")
}

fn pem_to_der(pem: &str) -> Result<Vec<u8>, String> {
    let b64: String = pem.lines().filter(|l| !l.starts_with("-----")).collect();
    base64_decode(&b64).ok_or_else(|| "PEM no válido".to_string())
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let bytes: Vec<u8> = s.bytes().filter(|c| !c.is_ascii_whitespace()).collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let mut acc = 0u32;
        let mut n = 0;
        for &c in chunk {
            if c == b'=' {
                break;
            }
            acc = (acc << 6) | val(c)?;
            n += 1;
        }
        acc <<= 6 * (4 - n);
        let b = acc.to_be_bytes();
        out.extend_from_slice(&b[1..1 + (n * 6 / 8)]);
    }
    Some(out)
}

/// Carga o crea la CA y el certificado del servidor para `ip`.
/// El certificado del servidor se regenera si la IP cambia o le quedan < 30 días.
pub fn load_or_create(dir: &Path, ip: Ipv4Addr) -> Result<TlsMaterial, String> {
    let io = |e: std::io::Error| format!("certificados: {e}");
    let ca_key_p = dir.join("ca-key.pem");
    let ca_crt_p = dir.join("ca.pem");
    let key_p = dir.join("server-key.pem");
    let crt_p = dir.join("server.pem");
    let meta_p = dir.join("server.meta");

    let (ca_key, ca_pem) = match (std::fs::read_to_string(&ca_key_p), std::fs::read_to_string(&ca_crt_p)) {
        (Ok(k), Ok(c)) => (KeyPair::from_pem(&k).map_err(|e| format!("clave de la CA: {e}"))?, c),
        _ => {
            let key = KeyPair::generate().map_err(|e| format!("generar clave CA: {e}"))?;
            let cert = ca_params().self_signed(&key).map_err(|e| format!("CA: {e}"))?;
            write_private(&ca_key_p, &key.serialize_pem()).map_err(io)?;
            std::fs::write(&ca_crt_p, cert.pem()).map_err(io)?;
            (key, cert.pem())
        }
    };

    let want_meta = format!("{ip}");
    let reuse = match (std::fs::read_to_string(&meta_p), std::fs::metadata(&crt_p)) {
        (Ok(m), Ok(md)) => {
            let fresh = md
                .modified()
                .ok()
                .and_then(|t| t.elapsed().ok())
                .map(|age| age.as_secs() < 367 * 86400)
                .unwrap_or(false);
            m.trim() == want_meta && fresh && key_p.exists()
        }
        _ => false,
    };

    let (cert_pem, key_pem) = if reuse {
        (std::fs::read_to_string(&crt_p).map_err(io)?, std::fs::read_to_string(&key_p).map_err(io)?)
    } else {
        let key = KeyPair::generate().map_err(|e| format!("generar clave: {e}"))?;
        let mut p = CertificateParams::default();
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, format!("{ip}"));
        p.distinguished_name = dn;
        p.subject_alt_names = vec![SanType::IpAddress(IpAddr::V4(ip))];
        p.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        p.not_before = OffsetDateTime::now_utc() - Duration::days(1);
        p.not_after = OffsetDateTime::now_utc() + Duration::days(397);
        p.use_authority_key_identifier_extension = true;
        let issuer = Issuer::new(ca_params(), &ca_key);
        let cert = p.signed_by(&key, &issuer).map_err(|e| format!("firmar certificado: {e}"))?;
        // Cadena completa: servidor + CA.
        let chain = format!("{}{}", cert.pem(), ca_pem);
        write_private(&key_p, &key.serialize_pem()).map_err(io)?;
        std::fs::write(&crt_p, &chain).map_err(io)?;
        std::fs::write(&meta_p, &want_meta).map_err(io)?;
        (chain, key.serialize_pem())
    };

    let ca_der = pem_to_der(&ca_pem)?;
    let first = cert_pem.split("-----END CERTIFICATE-----").next().unwrap_or("");
    let cert_der = pem_to_der(first)?;
    Ok(TlsMaterial {
        ca_fingerprint: fingerprint(&ca_der),
        cert_fingerprint: fingerprint(&cert_der),
        cert_pem,
        key_pem,
        ca_der,
    })
}

/// Escribe un archivo con la clave privada. En Windows, %APPDATA% ya solo es
/// accesible por el usuario y los administradores; en Unix se fija 0600.
fn write_private(path: &Path, data: &str) -> std::io::Result<()> {
    std::fs::write(path, data)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Perfil de configuración de iOS (.mobileconfig) que instala la CA.
pub fn ios_profile(ca_der: &[u8]) -> String {
    let b64 = base64_encode(ca_der);
    let uuid1 = uuid_from(ca_der, 1);
    let uuid2 = uuid_from(ca_der, 2);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>PayloadContent</key>
  <array>
    <dict>
      <key>PayloadCertificateFileName</key><string>vab-ca.cer</string>
      <key>PayloadContent</key><data>{b64}</data>
      <key>PayloadDescription</key><string>Certificado de la CA local de Virtual Audio Bridge (solo válido para tu PC en la red local).</string>
      <key>PayloadDisplayName</key><string>{CA_NAME}</string>
      <key>PayloadIdentifier</key><string>local.virtualaudiobridge.ca.cert</string>
      <key>PayloadType</key><string>com.apple.security.root</string>
      <key>PayloadUUID</key><string>{uuid1}</string>
      <key>PayloadVersion</key><integer>1</integer>
    </dict>
  </array>
  <key>PayloadDisplayName</key><string>Virtual Audio Bridge</string>
  <key>PayloadIdentifier</key><string>local.virtualaudiobridge.ca</string>
  <key>PayloadRemovalDisallowed</key><false/>
  <key>PayloadType</key><string>Configuration</string>
  <key>PayloadUUID</key><string>{uuid2}</string>
  <key>PayloadVersion</key><integer>1</integer>
</dict>
</plist>
"#
    )
}

fn uuid_from(seed: &[u8], n: u8) -> String {
    let mut data = seed.to_vec();
    data.push(n);
    let d = ring::digest::digest(&ring::digest::SHA256, &data);
    let b = d.as_ref();
    format!(
        "{:02X}{:02X}{:02X}{:02X}-{:02X}{:02X}-4{:01X}{:02X}-8{:01X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6] & 0xF, b[7], b[8] & 0xF, b[9], b[10], b[11], b[12], b[13], b[14], b[15]
    )
}

pub fn base64_encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for c in data.chunks(3) {
        let n = match c.len() {
            3 => (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32,
            2 => (c[0] as u32) << 16 | (c[1] as u32) << 8,
            _ => (c[0] as u32) << 16,
        };
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_roundtrip() {
        for n in 0..50u8 {
            let d: Vec<u8> = (0..n).map(|i| i.wrapping_mul(37)).collect();
            assert_eq!(base64_decode(&base64_encode(&d)).unwrap(), d);
        }
    }

    #[test]
    fn creates_chain_and_reuses_it() {
        let dir = std::env::temp_dir().join(format!("vab-tls-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ip: Ipv4Addr = "192.168.1.50".parse().unwrap();
        let a = load_or_create(&dir, ip).unwrap();
        assert_eq!(a.cert_pem.matches("BEGIN CERTIFICATE").count(), 2);
        let b = load_or_create(&dir, ip).unwrap();
        assert_eq!(a.cert_fingerprint, b.cert_fingerprint);
        // Cambio de IP → nuevo certificado de servidor, misma CA.
        let c = load_or_create(&dir, "192.168.1.51".parse().unwrap()).unwrap();
        assert_ne!(a.cert_fingerprint, c.cert_fingerprint);
        assert_eq!(a.ca_fingerprint, c.ca_fingerprint);
        // rustls acepta la clave y la cadena.
        let certs: Vec<_> = rustls_pemfile_certs(&c.cert_pem);
        assert_eq!(certs.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn rustls_pemfile_certs(pem: &str) -> Vec<Vec<u8>> {
        pem.split("-----END CERTIFICATE-----")
            .filter(|s| s.contains("BEGIN CERTIFICATE"))
            .map(|s| pem_to_der(&format!("{s}-----END CERTIFICATE-----")).unwrap())
            .collect()
    }
}
