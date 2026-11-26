//! Virtual Audio Bridge — motor del PC.
//!
//! Arranca, por este orden: configuración y registro, CA/certificado local,
//! hilo WebRTC (UDP), hilos de audio (WASAPI), servidor HTTPS para el móvil y
//! la interfaz local del PC (http://127.0.0.1:<admin_port>).
//!
//! En Windows se ejecuta sin ventana de consola (proceso en segundo plano).
//! Si ya hay una instancia, la segunda solo abre la interfaz y sale.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod audio;
mod auth;
mod config;
mod dsp;
mod log;
mod net;
mod rtc;
mod server;
mod stats;
mod tls;

use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

use config::Config;
use server::App;

fn usage() -> ! {
    eprintln!(
        "Virtual Audio Bridge {}\n\n\
         Uso: vab-engine [opciones]\n\
           --list-devices        Lista los dispositivos de reproducción de Windows y sale\n\
           --no-browser          No abre la interfaz en el navegador al arrancar\n\
           --print-admin-url     Muestra la URL de la interfaz local (con su token)\n\n\
         Configuración: {}",
        env!("CARGO_PKG_VERSION"),
        Config::path().display()
    );
    std::process::exit(2)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Sin ventana de consola propia: con opciones de línea de comandos se usa la
    // consola desde la que se lanzó (para --list-devices y mensajes de uso).
    #[cfg(windows)]
    if !args.is_empty() {
        use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
        let _ = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
    }
    let mut open_browser = true;
    let mut print_admin = false;
    for a in &args {
        match a.as_str() {
            "--list-devices" => {
                match audio::list_render_devices() {
                    Ok(list) => {
                        for d in list {
                            println!("{}{}  [{}]", if d.is_default { "* " } else { "  " }, d.name, d.state);
                        }
                    }
                    Err(e) => {
                        eprintln!("{e}");
                        std::process::exit(1);
                    }
                }
                return;
            }
            "--no-browser" => open_browser = false,
            "--print-admin-url" => print_admin = true,
            _ => usage(),
        }
    }

    let log = log::Log::new(Some(config::data_dir().join("vab.log")));
    if let Err(e) = run(log.clone(), open_browser, print_admin) {
        log.error(&e);
        std::process::exit(1);
    }
}

fn run(log: log::Log, open_browser: bool, print_admin: bool) -> Result<(), String> {
    // Proveedores criptográficos (DTLS de WebRTC y TLS del servidor web).
    str0m::crypto::from_feature_flags().install_process_default();
    let _ = rustls::crypto::ring::default_provider().install_default();

    let cfg = Config::load()?;
    if !Config::path().exists() {
        let _ = cfg.save();
    }

    // Instancia única: si el puerto de la interfaz está ocupado, ya hay otra.
    let admin_addr = SocketAddr::from(([127, 0, 0, 1], cfg.admin_port));
    let admin_token = load_or_create_admin_token()?;
    let admin_url = format!("http://127.0.0.1:{}/#k={}", admin_addr.port(), admin_token);
    let admin_listener = match std::net::TcpListener::bind(admin_addr) {
        Ok(l) => l,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            log.info("Virtual Audio Bridge ya está en ejecución: se abre su interfaz");
            if open_browser {
                open_in_browser(&admin_url);
            }
            return Ok(());
        }
        Err(e) => return Err(format!("No se puede abrir la interfaz local en {admin_addr}: {e}")),
    };
    admin_listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let ip = net::choose_ip(&cfg.bind_ip)?;
    log.info(&format!("Virtual Audio Bridge {} — IP local {ip}", env!("CARGO_PKG_VERSION")));
    for (name, a) in net::private_ipv4s() {
        log.info(&format!("  interfaz {name}: {a}"));
    }

    let tls = tls::load_or_create(&config::data_dir(), ip)?;
    let udp = UdpSocket::bind(SocketAddr::new(ip.into(), cfg.udp_port))
        .map_err(|e| format!("No se puede abrir UDP {ip}:{}: {e}", cfg.udp_port))?;

    let stats = Arc::new(stats::Stats::default());
    let control = Arc::new(audio::Control::default());
    let (rtc_tx, rtc_rx) = std::sync::mpsc::channel();
    let (net_ends, dev_ends) = audio::make_pipes();
    drop(dev_ends); // los hilos de audio reciben sus propias colas en restart_audio()
    let params = rtc::RtcParams {
        bitrate: cfg.opus_bitrate,
        frame_ms: cfg.opus_frame_ms,
        jitter_min_ms: cfg.jitter_min_ms,
        jitter_max_ms: cfg.jitter_max_ms,
    };
    let rtc_thread = rtc::spawn(udp, rtc_rx, net_ends, control.clone(), stats.clone(), log.clone(), params);

    let origin = format!("https://{ip}:{}", cfg.https_port);
    let phone_addr = SocketAddr::new(ip.into(), cfg.https_port);
    let app = Arc::new(App {
        auth: Mutex::new(auth::Auth::with_store(config::data_dir().join("sessions.txt"))),
        rtc_tx,
        stats,
        control,
        log: log.clone(),
        config: Mutex::new(cfg),
        tls,
        origin: origin.clone(),
        admin_token,
        phone: Mutex::new(Default::default()),
        ws_epoch: AtomicU64::new(0),
        kick_reason: Mutex::new("replaced"),
        devices: Mutex::new(Default::default()),
        audio: Mutex::new(None),
        started: std::time::Instant::now(),
        quit: tokio::sync::Notify::new(),
    });
    app.restart_audio();

    log.info(&format!("Página del móvil: {origin}  (escanea el QR de la interfaz del PC)"));
    log.info(&format!("Huella de la CA local: {}", app.tls.ca_fingerprint));
    if print_admin {
        println!("{admin_url}");
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let res = rt.block_on(async {
        let a1 = app.clone();
        let a2 = app.clone();
        let phone = tokio::spawn(async move { server::phone::serve(a1, phone_addr).await });
        let listener = tokio::net::TcpListener::from_std(admin_listener).map_err(|e| e.to_string())?;
        let admin = tokio::spawn(async move { server::admin::serve(a2, listener).await });
        if open_browser {
            open_in_browser(&admin_url);
        }
        tokio::select! {
            r = phone => r.map_err(|e| e.to_string())?,
            r = admin => r.map_err(|e| e.to_string())?,
            _ = tokio::signal::ctrl_c() => { log.info("Cerrando…"); Ok(()) }
            _ = app.quit.notified() => { log.info("Cerrando a petición del usuario…"); Ok(()) }
        }
    });
    let _ = app.rtc_tx.send(rtc::RtcCmd::Shutdown);
    if let Some(t) = app.audio.lock().unwrap().take() {
        t.stop();
    }
    let _ = rtc_thread.join();
    res
}

/// Token de la interfaz local. Se guarda en el directorio de datos del usuario
/// para que una segunda instancia (o el acceso directo) pueda abrir la interfaz.
fn load_or_create_admin_token() -> Result<String, String> {
    let p = config::data_dir().join("admin-token");
    if let Ok(t) = std::fs::read_to_string(&p) {
        let t = t.trim().to_string();
        if t.len() == 32 && t.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Ok(t);
        }
    }
    let t = auth::random_hex(16);
    std::fs::write(&p, &t).map_err(|e| format!("No se puede guardar el token de la interfaz: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
    }
    Ok(t)
}

fn open_in_browser(url: &str) {
    #[cfg(windows)]
    {
        // `start` con título vacío; la URL va entre comillas por el carácter '#'.
        let _ = std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = url;
    }
}
