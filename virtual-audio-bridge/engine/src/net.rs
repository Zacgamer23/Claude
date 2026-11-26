//! Selección de la dirección de red local.
//!
//! Solo se usan direcciones IPv4 privadas (RFC 1918). El servidor escucha en
//! esa IP concreta y no en 0.0.0.0, para no exponerse en otras interfaces
//! (VPN, adaptadores virtuales, redes públicas).

use std::net::{IpAddr, Ipv4Addr};

pub fn is_private_v4(ip: &Ipv4Addr) -> bool {
    ip.is_private() && !ip.is_loopback() && !ip.is_link_local()
}

/// Interfaces con IPv4 privada: (nombre, ip).
pub fn private_ipv4s() -> Vec<(String, Ipv4Addr)> {
    let mut v: Vec<(String, Ipv4Addr)> = local_ip_address::list_afinet_netifas()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(name, ip)| match ip {
            IpAddr::V4(v4) if is_private_v4(&v4) => Some((name, v4)),
            _ => None,
        })
        .collect();
    // Preferencia: Wi-Fi/WLAN, luego 192.168.x, luego el resto. Se evitan
    // adaptadores virtuales habituales (Hyper-V, WSL, VirtualBox, VMware, Docker).
    let score = |(name, ip): &(String, Ipv4Addr)| {
        let n = name.to_lowercase();
        let mut s = 0i32;
        if n.contains("wi-fi") || n.contains("wifi") || n.contains("wlan") || n.contains("wireless") {
            s += 100;
        }
        if ip.octets()[0] == 192 && ip.octets()[1] == 168 {
            s += 10;
        }
        if ["vethernet", "wsl", "virtualbox", "vmware", "docker", "hyper-v", "loopback", "tailscale", "zerotier"]
            .iter()
            .any(|k| n.contains(k))
        {
            s -= 1000;
        }
        -s
    };
    v.sort_by_key(score);
    v
}

pub fn choose_ip(configured: &str) -> Result<Ipv4Addr, String> {
    if !configured.is_empty() {
        let ip: Ipv4Addr = configured.parse().map_err(|_| format!("bind_ip no válida: {configured}"))?;
        // La compilación de pruebas (test-io) admite cualquier IP para poder
        // ejecutarse en contenedores de CI sin red local privada.
        if !is_private_v4(&ip) && !cfg!(feature = "test-io") {
            return Err(format!("{ip} no es una dirección de red local privada"));
        }
        return Ok(ip);
    }
    private_ipv4s()
        .first()
        .map(|(_, ip)| *ip)
        .ok_or_else(|| "No se ha encontrado ninguna dirección IPv4 de red local. ¿Está el PC conectado a la Wi-Fi?".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_ranges() {
        assert!(is_private_v4(&"192.168.1.20".parse().unwrap()));
        assert!(is_private_v4(&"10.0.0.5".parse().unwrap()));
        assert!(is_private_v4(&"172.16.3.4".parse().unwrap()));
        assert!(!is_private_v4(&"8.8.8.8".parse().unwrap()));
        assert!(!is_private_v4(&"127.0.0.1".parse().unwrap()));
        assert!(!is_private_v4(&"169.254.1.1".parse().unwrap()));
    }
    #[test]
    fn rejects_public_configured_ip() {
        assert!(choose_ip("8.8.8.8").is_err());
        assert_eq!(choose_ip("192.168.0.9").unwrap(), "192.168.0.9".parse::<Ipv4Addr>().unwrap());
    }
}
