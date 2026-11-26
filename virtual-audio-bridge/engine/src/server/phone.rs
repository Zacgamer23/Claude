//! Servidor HTTPS para el móvil: página web, emparejamiento y señalización WSS.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering::{Relaxed, SeqCst};
use std::time::Duration;

use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};

use super::{App, json_err, secure_headers};
use crate::auth::PairError;
use crate::rtc::RtcCmd;

const INDEX: &str = include_str!("../../../web/phone/index.html");
const APP_JS: &str = include_str!("../../../web/phone/app.js");
const STYLE: &str = include_str!("../../../web/phone/style.css");

const COOKIE: &str = "vab_s";

pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/app.js", get(|| async { asset(APP_JS, "text/javascript; charset=utf-8") }))
        .route("/style.css", get(|| async { asset(STYLE, "text/css; charset=utf-8") }))
        .route("/ca.mobileconfig", get(ca_profile))
        .route("/ca.cer", get(ca_cer))
        .route("/api/pair", post(pair))
        .route("/api/session", get(session))
        .route("/ws", get(ws))
        .with_state(app)
}

fn asset(body: &'static str, ct: &'static str) -> Response {
    let mut r = body.into_response();
    r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static(ct));
    r.headers_mut().insert("Cache-Control", HeaderValue::from_static("no-store"));
    r
}

async fn index(State(app): State<Arc<App>>) -> Response {
    let mut r = axum::response::Html(INDEX).into_response();
    r.headers_mut().insert("Permissions-Policy", HeaderValue::from_static("microphone=(self), camera=(), geolocation=()"));
    let wss = app.origin.replace("https://", "wss://");
    secure_headers(r, &wss)
}

async fn ca_profile(State(app): State<Arc<App>>) -> Response {
    let mut r = crate::tls::ios_profile(&app.tls.ca_der).into_response();
    r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/x-apple-aspen-config"));
    r.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"VirtualAudioBridge-CA.mobileconfig\""),
    );
    r
}

async fn ca_cer(State(app): State<Arc<App>>) -> Response {
    let mut r = app.tls.ca_der.clone().into_response();
    r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/x-x509-ca-cert"));
    r
}

fn cookie_session(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    raw.split(';').map(|p| p.trim()).find_map(|p| p.strip_prefix(&format!("{COOKIE}=")).map(|s| s.to_string()))
}

fn same_origin(app: &App, headers: &HeaderMap) -> bool {
    headers.get(header::ORIGIN).and_then(|o| o.to_str().ok()) == Some(app.origin.as_str())
}

#[derive(serde::Deserialize)]
struct PairReq {
    token: String,
}

async fn pair(State(app): State<Arc<App>>, headers: HeaderMap, body: axum::Json<PairReq>) -> Response {
    if !same_origin(&app, &headers) {
        return json_err(StatusCode::FORBIDDEN, "Origen no permitido");
    }
    let res = app.auth.lock().unwrap().pair(body.token.trim());
    match res {
        Ok(session) => {
            app.log.info("Móvil emparejado correctamente");
            let mut r = axum::Json(serde_json::json!({ "ok": true })).into_response();
            let c = format!("{COOKIE}={session}; Path=/; Max-Age=2592000; HttpOnly; Secure; SameSite=Strict");
            if let Ok(v) = HeaderValue::from_str(&c) {
                r.headers_mut().insert(header::SET_COOKIE, v);
            }
            r
        }
        Err(e) => {
            let msg = match e {
                PairError::Invalid => "Código de emparejamiento no válido o ya usado. Escanea el QR actual del PC.",
                PairError::Expired => "El código ha caducado. Escanea el QR actual del PC.",
                PairError::Locked => "Demasiados intentos. Genera un código nuevo en el PC.",
            };
            app.log.warn(&format!("Intento de emparejamiento rechazado: {msg}"));
            json_err(StatusCode::FORBIDDEN, msg)
        }
    }
}

async fn session(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    let ok = cookie_session(&headers).map(|s| app.auth.lock().unwrap().session_valid(&s)).unwrap_or(false);
    axum::Json(serde_json::json!({ "ok": ok })).into_response()
}

async fn ws(State(app): State<Arc<App>>, headers: HeaderMap, upgrade: WebSocketUpgrade) -> Response {
    if !same_origin(&app, &headers) {
        return json_err(StatusCode::FORBIDDEN, "Origen no permitido");
    }
    let ok = cookie_session(&headers).map(|s| app.auth.lock().unwrap().session_valid(&s)).unwrap_or(false);
    if !ok {
        return json_err(StatusCode::UNAUTHORIZED, "Sesión no válida: vuelve a escanear el QR del PC");
    }
    let ua = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    upgrade.max_message_size(256 * 1024).on_upgrade(move |sock| handle_ws(app, sock, ua))
}

async fn handle_ws(app: Arc<App>, mut sock: WebSocket, ua: String) {
    *app.kick_reason.lock().unwrap() = "replaced";
    let epoch = app.ws_epoch.fetch_add(1, SeqCst) + 1;
    {
        let mut p = app.phone.lock().unwrap();
        p.ws_connected = true;
        p.user_agent = ua;
    }
    app.log.info("Página del móvil conectada (señalización)");
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        if app.ws_epoch.load(SeqCst) != epoch {
            let reason = *app.kick_reason.lock().unwrap();
            let msg = serde_json::json!({ "t": "bye", "reason": reason });
            let _ = sock.send(Message::Text(msg.to_string().into())).await;
            break;
        }
        tokio::select! {
            _ = tick.tick() => {
                let msg = serde_json::json!({
                    "t": "pcstats",
                    "stats": app.stats.snapshot(),
                    "bridge": app.control.bridge_enabled.load(Relaxed),
                    "devices": *app.devices.lock().unwrap(),
                    "ca_fp": app.tls.ca_fingerprint,
                });
                if sock.send(Message::Text(msg.to_string().into())).await.is_err() { break; }
            }
            m = sock.recv() => {
                let Some(Ok(m)) = m else { break };
                let Message::Text(txt) = m else { continue };
                let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) else { continue };
                match v["t"].as_str() {
                    Some("offer") => {
                        let reply = match serde_json::from_value::<str0m::change::SdpOffer>(v["desc"].clone()) {
                            Ok(offer) => {
                                let (tx, rx) = tokio::sync::oneshot::channel();
                                let _ = app.rtc_tx.send(RtcCmd::Offer { offer, reply: tx });
                                match tokio::time::timeout(Duration::from_secs(5), rx).await {
                                    Ok(Ok(Ok(answer))) => serde_json::json!({ "t": "answer", "desc": answer }),
                                    Ok(Ok(Err(e))) => serde_json::json!({ "t": "error", "msg": e }),
                                    _ => serde_json::json!({ "t": "error", "msg": "El motor WebRTC del PC no responde" }),
                                }
                            }
                            Err(e) => serde_json::json!({ "t": "error", "msg": format!("Oferta no válida: {e}") }),
                        };
                        if sock.send(Message::Text(reply.to_string().into())).await.is_err() { break; }
                    }
                    Some("hangup") => { let _ = app.rtc_tx.send(RtcCmd::Disconnect); }
                    Some("report") => { app.phone.lock().unwrap().report = v["data"].clone(); }
                    Some("log") => {
                        let msg = v["msg"].as_str().unwrap_or("").chars().take(500).collect::<String>();
                        match v["level"].as_str() {
                            Some("error") => app.log.error(&format!("Móvil: {msg}")),
                            Some("warn") => app.log.warn(&format!("Móvil: {msg}")),
                            _ => app.log.info(&format!("Móvil: {msg}")),
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    if app.ws_epoch.load(SeqCst) == epoch {
        app.phone.lock().unwrap().ws_connected = false;
        app.log.info("Página del móvil desconectada (señalización)");
    }
}

pub async fn serve(app: Arc<App>, addr: SocketAddr) -> Result<(), String> {
    let cfg = axum_server::tls_rustls::RustlsConfig::from_pem(
        app.tls.cert_pem.as_bytes().to_vec(),
        app.tls.key_pem.as_bytes().to_vec(),
    )
    .await
    .map_err(|e| format!("TLS: {e}"))?;
    axum_server::bind_rustls(addr, cfg)
        .serve(router(app).into_make_service())
        .await
        .map_err(|e| format!("Servidor HTTPS en {addr}: {e}"))
}
