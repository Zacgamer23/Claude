//! Interfaz local del PC (solo 127.0.0.1). Muestra el estado real del motor
//! y permite controlarlo. Las llamadas a la API exigen el token de
//! administración (generado al arrancar) y un `Host` local, para impedir que
//! una web externa la use mediante *DNS rebinding*.

use std::sync::Arc;
use std::sync::atomic::Ordering::{Relaxed, SeqCst};

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};

use super::{App, json_err, secure_headers};
use crate::rtc::RtcCmd;

const ADMIN_HTML: &str = include_str!("../../../web/admin/index.html");
const ADMIN_JS: &str = include_str!("../../../web/admin/admin.js");
const ADMIN_CSS: &str = include_str!("../../../web/admin/admin.css");

pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/", get(|| async { secure_headers(axum::response::Html(ADMIN_HTML).into_response(), "") }))
        .route("/admin.js", get(|| async { asset(ADMIN_JS, "text/javascript; charset=utf-8") }))
        .route("/admin.css", get(|| async { asset(ADMIN_CSS, "text/css; charset=utf-8") }))
        .route("/api/status", get(status))
        .route("/api/devices", get(devices))
        .route("/api/bridge", post(bridge))
        .route("/api/disconnect", post(disconnect))
        .route("/api/new-pairing", post(new_pairing))
        .route("/api/revoke", post(revoke))
        .route("/api/config", post(set_config))
        .route("/api/quit", post(quit))
        .with_state(app)
}

fn asset(body: &'static str, ct: &'static str) -> Response {
    let mut r = body.into_response();
    r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static(ct));
    r
}

fn authorized(app: &App, h: &HeaderMap) -> bool {
    let host_ok = h
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.starts_with("127.0.0.1:") || v.starts_with("localhost:"))
        .unwrap_or(false);
    let tok = h.get("x-admin-token").and_then(|v| v.to_str().ok()).unwrap_or("");
    host_ok
        && tok.len() == app.admin_token.len()
        && tok.bytes().zip(app.admin_token.bytes()).fold(0u8, |a, (x, y)| a | (x ^ y)) == 0
}

macro_rules! guard {
    ($app:expr, $h:expr) => {
        if !authorized(&$app, &$h) {
            return json_err(StatusCode::UNAUTHORIZED, "Token de administración no válido");
        }
    };
}

async fn status(State(app): State<Arc<App>>, h: HeaderMap) -> Response {
    guard!(app, h);
    let url = app.pairing_url();
    let qr = qrcode::QrCode::new(url.as_bytes())
        .map(|q| q.render::<qrcode::render::svg::Color>().min_dimensions(240, 240).quiet_zone(true).build())
        .unwrap_or_default();
    let (expires, sessions) = {
        let a = app.auth.lock().unwrap();
        (a.pairing_expires_in().map(|d| d.as_secs()).unwrap_or(0), a.session_count())
    };
    axum::Json(serde_json::json!({
        "ok": true,
        "uptime_s": app.started.elapsed().as_secs(),
        "origin": app.origin,
        "pairing_url": url,
        "pairing_expires_s": expires,
        "paired_sessions": sessions,
        "qr_svg": qr,
        "ca_fingerprint": app.tls.ca_fingerprint,
        "cert_fingerprint": app.tls.cert_fingerprint,
        "bridge": app.control.bridge_enabled.load(Relaxed),
        "stats": app.stats.snapshot(),
        "devices": *app.devices.lock().unwrap(),
        "phone": *app.phone.lock().unwrap(),
        "config": *app.config.lock().unwrap(),
        "log": app.log.recent(200),
        "test_build": cfg!(feature = "test-io"),
    }))
    .into_response()
}

async fn devices(State(app): State<Arc<App>>, h: HeaderMap) -> Response {
    guard!(app, h);
    match tokio::task::spawn_blocking(crate::audio::list_render_devices).await {
        Ok(Ok(list)) => axum::Json(serde_json::json!({ "ok": true, "devices": list })).into_response(),
        Ok(Err(e)) => json_err(StatusCode::INTERNAL_SERVER_ERROR, &e),
        Err(e) => json_err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

#[derive(serde::Deserialize)]
struct BridgeReq {
    enabled: bool,
}

async fn bridge(State(app): State<Arc<App>>, h: HeaderMap, b: axum::Json<BridgeReq>) -> Response {
    guard!(app, h);
    app.control.bridge_enabled.store(b.enabled, Relaxed);
    app.log.info(if b.enabled { "Puente de audio ACTIVADO" } else { "Puente de audio DETENIDO" });
    axum::Json(serde_json::json!({ "ok": true })).into_response()
}

async fn disconnect(State(app): State<Arc<App>>, h: HeaderMap) -> Response {
    guard!(app, h);
    let _ = app.rtc_tx.send(RtcCmd::Disconnect);
    *app.kick_reason.lock().unwrap() = "pc";
    app.ws_epoch.fetch_add(1, SeqCst);
    app.log.info("Móvil desconectado desde el PC");
    axum::Json(serde_json::json!({ "ok": true })).into_response()
}

async fn new_pairing(State(app): State<Arc<App>>, h: HeaderMap) -> Response {
    guard!(app, h);
    app.auth.lock().unwrap().new_pairing_token();
    app.log.info("Nuevo código de emparejamiento generado");
    axum::Json(serde_json::json!({ "ok": true })).into_response()
}

async fn revoke(State(app): State<Arc<App>>, h: HeaderMap) -> Response {
    guard!(app, h);
    app.auth.lock().unwrap().revoke_all();
    let _ = app.rtc_tx.send(RtcCmd::Disconnect);
    *app.kick_reason.lock().unwrap() = "revoked";
    app.ws_epoch.fetch_add(1, SeqCst);
    app.log.info("Todos los emparejamientos revocados");
    axum::Json(serde_json::json!({ "ok": true })).into_response()
}

#[derive(serde::Deserialize)]
struct ConfigReq {
    speaker_device: Option<String>,
    mic_sink_device: Option<String>,
    opus_bitrate: Option<i32>,
    opus_frame_ms: Option<u32>,
    jitter_min_ms: Option<u32>,
    jitter_max_ms: Option<u32>,
}

async fn set_config(State(app): State<Arc<App>>, h: HeaderMap, b: axum::Json<ConfigReq>) -> Response {
    guard!(app, h);
    let (devices_changed, saved) = {
        let mut c = app.config.lock().unwrap();
        let before = (c.speaker_device.clone(), c.mic_sink_device.clone());
        if let Some(v) = &b.speaker_device { c.speaker_device = v.trim().to_string(); }
        if let Some(v) = &b.mic_sink_device { c.mic_sink_device = v.trim().to_string(); }
        if let Some(v) = b.opus_bitrate { c.opus_bitrate = v; }
        if let Some(v) = b.opus_frame_ms { c.opus_frame_ms = v; }
        if let Some(v) = b.jitter_min_ms { c.jitter_min_ms = v; }
        if let Some(v) = b.jitter_max_ms { c.jitter_max_ms = v; }
        *c = c.clone().sanitized();
        (before != (c.speaker_device.clone(), c.mic_sink_device.clone()), c.save())
    };
    if let Err(e) = saved {
        return json_err(StatusCode::INTERNAL_SERVER_ERROR, &e);
    }
    if devices_changed {
        let a = app.clone();
        let _ = tokio::task::spawn_blocking(move || a.restart_audio()).await;
    }
    axum::Json(serde_json::json!({
        "ok": true,
        "note": "Los cambios de dispositivo se aplican al momento; los de códec y jitter, al reiniciar la aplicación."
    }))
    .into_response()
}

async fn quit(State(app): State<Arc<App>>, h: HeaderMap) -> Response {
    guard!(app, h);
    app.quit.notify_one();
    axum::Json(serde_json::json!({ "ok": true })).into_response()
}

pub async fn serve(app: Arc<App>, listener: tokio::net::TcpListener) -> Result<(), String> {
    axum::serve(listener, router(app)).await.map_err(|e| e.to_string())
}
