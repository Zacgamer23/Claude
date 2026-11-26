// Virtual Audio Bridge — página del móvil.
// Transporte: WebRTC (Opus sobre SRTP/UDP) directo con el PC en la red local.
// Señalización: WebSocket seguro con el mismo servidor del PC. Sin STUN/TURN.
'use strict';

const $ = (id) => document.getElementById(id);
const S = {
  ws: null, wsRetry: 0, pc: null, mic: null, running: false,
  wakeLock: null, statsTimer: null, iceTimer: null, reconnectTimer: null,
  lastStats: null, pcStats: null, inputClass: 'none', inputLabel: '',
};

// ---------- Utilidades de interfaz ----------
function pill(id, text, cls) { const el = $(id); el.textContent = text; el.className = 'pill ' + (cls || ''); }
function showError(msg) { $('error-text').textContent = msg; $('error-card').hidden = !msg; if (msg) logPC('error', msg); }
function logPC(level, msg) {
  try { if (S.ws && S.ws.readyState === 1) S.ws.send(JSON.stringify({ t: 'log', level, msg })); } catch (_) {}
}
function fmtMs(v) { return v == null || isNaN(v) ? '—' : `${Math.round(v)} ms`; }
function mode() { return document.querySelector('input[name=mode]:checked').value; }
function setRunningUI(r) {
  $('btn-start').hidden = r; $('btn-stop').hidden = !r; $('btn-reconnect').hidden = !r;
  document.querySelectorAll('input[name=mode]').forEach((e) => (e.disabled = r));
}

// ---------- Comprobaciones del entorno ----------
function checkEnvironment() {
  if (!window.isSecureContext) {
    showError('La página no está en un contexto seguro (HTTPS). Ábrela con la dirección https:// que muestra el PC.');
    return false;
  }
  if (!navigator.mediaDevices || !navigator.mediaDevices.getUserMedia || !window.RTCPeerConnection) {
    showError('Este navegador no permite audio en tiempo real (falta getUserMedia o WebRTC). Usa Safari actualizado.');
    return false;
  }
  return true;
}

// ---------- Emparejamiento ----------
async function pairIfNeeded() {
  const m = location.hash.match(/[#&]t=([0-9a-f]{32})/);
  if (m) {
    history.replaceState(null, '', '/'); // el token no queda en el historial
    const r = await fetch('/api/pair', {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ token: m[1] }), credentials: 'same-origin',
    });
    const j = await r.json().catch(() => ({}));
    if (!r.ok) { showError(j.error || 'No se pudo emparejar con el PC.'); return false; }
    return true;
  }
  const r = await fetch('/api/session', { credentials: 'same-origin' });
  const j = await r.json().catch(() => ({}));
  if (!j.ok) { showError('Este móvil no está emparejado. Escanea el código QR que muestra la aplicación del PC.'); return false; }
  return true;
}

// ---------- Señalización ----------
async function connectWS() {
  // Si el PC ya no reconoce la sesión (revocada), no tiene sentido reintentar.
  try {
    const j = await (await fetch('/api/session', { credentials: 'same-origin' })).json();
    if (!j.ok) { showError('Este móvil ya no está emparejado. Escanea el código QR del PC.'); stop(false); return; }
  } catch (_) {
    pill('st-ws', 'PC no disponible', 'bad');
    setTimeout(connectWS, Math.min(10000, 500 * 2 ** S.wsRetry++));
    return;
  }
  const ws = new WebSocket(`wss://${location.host}/ws`);
  S.ws = ws;
  pill('st-ws', 'conectando…', 'warn');
  ws.onopen = () => {
    S.wsRetry = 0; pill('st-ws', 'conectado', 'ok');
    // Tras un reinicio del PC la sesión WebRTC anterior ya no existe: se renegocia.
    if (S.running && (!S.pc || S.pc.connectionState !== 'connected')) negotiate().catch((e) => showError(e.message));
  };
  ws.onmessage = (ev) => {
    if (S.ws !== ws) return; // mensaje de una conexión antigua
    let m; try { m = JSON.parse(ev.data); } catch (_) { return; }
    if (m.t === 'answer') onAnswer(m.desc);
    else if (m.t === 'error') showError('PC: ' + m.msg);
    else if (m.t === 'pcstats') onPcStats(m);
    else if (m.t === 'bye') {
      const why = {
        pc: 'Desconectado desde el PC. Pulsa «Iniciar puente» para volver a conectar.',
        revoked: 'El PC ha revocado el emparejamiento. Escanea de nuevo el código QR.',
        replaced: 'Otra pestaña o dispositivo ha tomado el control del puente.',
      }[m.reason] || 'El PC ha cerrado la conexión.';
      S.kicked = true; stop(false); showError(why);
    }
  };
  ws.onclose = (ev) => {
    pill('st-ws', 'desconectado', 'bad');
    if (S.ws !== ws) return;
    S.ws = null;
    if (S.kicked) { S.kicked = false; return; }
    const delay = Math.min(10000, 500 * 2 ** S.wsRetry++);
    setTimeout(connectWS, delay);
  };
}
function send(obj) { if (S.ws && S.ws.readyState === 1) { S.ws.send(JSON.stringify(obj)); return true; } return false; }

// ---------- Entrada de audio (micrófono de los auriculares) ----------
const RE_BUILTIN = /^(iphone|micr[oó]fono del iphone|iphone microphone|built-?in.*|internal.*|default|predeterminado|speakerphone|earpiece|front|back|bottom|top)$/i;
const RE_BT = /bluetooth|airpods|beats|nothing|\bear\s*\(|headset|hands-?free|auricular|buds|hfp|bt\b/i;

function classifyLabel(label) {
  const l = (label || '').trim();
  if (!l) return 'unknown';
  if (RE_BT.test(l)) return 'bluetooth';
  if (RE_BUILTIN.test(l) || /iphone/i.test(l)) return 'builtin';
  return 'unknown';
}

async function openMic(deviceId) {
  const voice = $('opt-voice').checked;
  const audio = { echoCancellation: voice, noiseSuppression: voice, autoGainControl: voice, channelCount: 1 };
  if (deviceId) audio.deviceId = { exact: deviceId };
  const stream = await navigator.mediaDevices.getUserMedia({ audio, video: false });
  const track = stream.getAudioTracks()[0];
  S.inputLabel = track.label || '';
  S.inputClass = classifyLabel(S.inputLabel);

  // Lista de entradas disponibles (con permiso concedido ya trae etiquetas).
  const inputs = (await navigator.mediaDevices.enumerateDevices()).filter((d) => d.kind === 'audioinput');
  const sel = $('input-select');
  sel.innerHTML = '';
  for (const d of inputs) {
    const o = document.createElement('option');
    o.value = d.deviceId; o.textContent = d.label || '(sin nombre)';
    if (d.deviceId === track.getSettings().deviceId) o.selected = true;
    sel.appendChild(o);
  }
  $('input-select-box').hidden = inputs.length < 2;
  return track;
}

/** Devuelve true si se puede transmitir el micrófono sin sustituirlo en silencio. */
function inputAllowed() {
  const label = S.inputLabel || '(sin nombre)';
  if (S.inputClass === 'bluetooth') {
    pill('st-input', label, 'ok');
    $('confirm-box').hidden = true;
    return true;
  }
  if (S.inputClass === 'builtin') {
    pill('st-input', label + ' (interno)', 'bad');
    $('confirm-box').hidden = true;
    showError(`El iPhone está usando su micrófono interno («${label}»), no el de los auriculares. ` +
      'No se transmitirá. Comprueba que los auriculares están conectados y seleccionados en el Centro de control, ' +
      'elige otro micrófono en la lista si aparece, o usa el modo «Solo escucha».');
    return false;
  }
  // iOS a veces no expone qué entrada usa: se exige confirmación explícita.
  pill('st-input', label + ' (sin verificar)', 'warn');
  $('confirm-text').textContent = `El navegador identifica la entrada como «${label}» y no permite comprobar si es la de los auriculares.`;
  $('confirm-box').hidden = false;
  if (!$('confirm-route').checked) {
    showError('Confirma la ruta de audio antes de transmitir el micrófono (casilla de arriba) y pulsa «Iniciar puente» de nuevo.');
    return false;
  }
  return true;
}

// ---------- WebRTC ----------
function mungeOffer(sdp) {
  // Opus: estéreo en recepción (el PC envía estéreo) y FEC en banda.
  const m = sdp.match(/a=rtpmap:(\d+) opus\/48000\/2/i);
  if (!m) return sdp;
  const pt = m[1];
  return sdp.replace(new RegExp(`a=fmtp:${pt} ([^\\r\\n]*)`), (all, params) => {
    const p = new Map(params.split(';').map((kv) => kv.trim().split('=')).filter((a) => a[0]));
    p.set('stereo', '1'); p.set('useinbandfec', '1');
    if (mode() === 'call') p.set('sprop-stereo', '0');
    return `a=fmtp:${pt} ` + [...p].map(([k, v]) => `${k}=${v}`).join(';');
  });
}

function applyJitterTarget(receiver) {
  const ms = Math.max(0, Number($('opt-jb').value) || 0);
  try {
    if ('jitterBufferTarget' in receiver) receiver.jitterBufferTarget = ms;
    else if ('playoutDelayHint' in receiver) receiver.playoutDelayHint = ms / 1000;
  } catch (e) { logPC('warn', 'jitterBufferTarget no aplicado: ' + e.message); }
}

async function negotiate() {
  closePC();
  const pc = new RTCPeerConnection({ iceServers: [], bundlePolicy: 'max-bundle', rtcpMuxPolicy: 'require' });
  S.pc = pc;
  if (mode() === 'call' && S.mic) pc.addTransceiver(S.mic, { direction: 'sendrecv' });
  else pc.addTransceiver('audio', { direction: 'recvonly' });

  pc.ontrack = (ev) => {
    applyJitterTarget(ev.receiver);
    const el = $('remote');
    el.srcObject = new MediaStream([ev.track]);
    el.play().catch((e) => showError('Safari ha bloqueado la reproducción (' + e.name + '). Pulsa «Reconectar».'));
  };
  pc.onconnectionstatechange = () => {
    const s = pc.connectionState;
    const cls = { connected: 'ok', connecting: 'warn', new: 'warn', disconnected: 'warn', failed: 'bad', closed: 'bad' }[s];
    pill('st-rtc', { connected: 'conectado', connecting: 'conectando…', new: 'iniciando', disconnected: 'interrumpido', failed: 'fallo', closed: 'cerrado' }[s] || s, cls);
    if (s === 'connected') { clearTimeout(S.iceTimer); showError(''); logPC('info', 'WebRTC conectado'); }
    if (s === 'failed') scheduleReconnect(500);
    if (s === 'disconnected') scheduleReconnect(3000);
  };

  const offer = await pc.createOffer();
  await pc.setLocalDescription({ type: 'offer', sdp: mungeOffer(offer.sdp) });
  if (!send({ t: 'offer', desc: pc.localDescription })) {
    showError('Sin conexión con el PC: no se puede negociar el audio. Reintentando…');
    return;
  }
  clearTimeout(S.iceTimer);
  S.iceTimer = setTimeout(() => {
    if (S.pc === pc && pc.connectionState !== 'connected') {
      showError('El audio no consigue conectar por UDP con el PC. Revisa: (1) en Windows la red Wi-Fi debe ser «Privada», ' +
        '(2) el firewall debe permitir Virtual Audio Bridge, (3) el router no debe aislar a los clientes Wi-Fi (red de invitados).');
    }
  }, 15000);
}

async function onAnswer(desc) {
  if (!S.pc) return;
  try { await S.pc.setRemoteDescription(desc); }
  catch (e) { showError('Respuesta del PC no válida: ' + e.message); }
}

function closePC() {
  clearTimeout(S.iceTimer);
  if (S.pc) { try { S.pc.close(); } catch (_) {} S.pc = null; }
}

function scheduleReconnect(ms) {
  if (!S.running) return;
  clearTimeout(S.reconnectTimer);
  S.reconnectTimer = setTimeout(() => {
    if (!S.running) return;
    if (S.pc && S.pc.connectionState === 'connected') return;
    logPC('warn', 'Reconectando WebRTC');
    negotiate().catch((e) => showError('Reconexión fallida: ' + e.message));
  }, ms);
}

// ---------- Pantalla encendida / segundo plano ----------
async function keepAwake() {
  if (!('wakeLock' in navigator)) { pill('st-vis', 'sin bloqueo de pantalla', 'warn'); return; }
  try {
    S.wakeLock = await navigator.wakeLock.request('screen');
    pill('st-vis', 'se mantendrá encendida', 'ok');
    S.wakeLock.addEventListener('release', () => { if (S.running) pill('st-vis', 'puede apagarse', 'warn'); });
  } catch (e) { pill('st-vis', 'no se pudo mantener', 'warn'); logPC('warn', 'Wake Lock: ' + e.message); }
}
document.addEventListener('visibilitychange', () => {
  const v = document.visibilityState;
  logPC(v === 'hidden' ? 'warn' : 'info', v === 'hidden' ? 'La página pasó a segundo plano o se bloqueó la pantalla' : 'La página volvió al primer plano');
  if (v === 'visible' && S.running) {
    keepAwake();
    if (!S.pc || ['failed', 'closed', 'disconnected'].includes(S.pc.connectionState)) scheduleReconnect(100);
    if (S.mic && S.mic.readyState === 'ended') { showError('iOS ha cerrado el micrófono al pasar a segundo plano. Pulsa «Reconectar».'); }
  }
  sendReport();
});

// ---------- Estadísticas reales ----------
async function collectStats() {
  if (!S.pc) return null;
  const rep = await S.pc.getStats();
  const out = { t: Date.now() };
  rep.forEach((s) => {
    if (s.type === 'inbound-rtp' && s.kind === 'audio') {
      out.jbDelayMs = s.jitterBufferEmittedCount ? (1000 * s.jitterBufferDelay) / s.jitterBufferEmittedCount : null;
      out.jbTargetMs = s.jitterBufferEmittedCount && s.jitterBufferTargetDelay != null ? (1000 * s.jitterBufferTargetDelay) / s.jitterBufferEmittedCount : null;
      out.packetsReceived = s.packetsReceived; out.packetsLost = s.packetsLost;
      out.concealed = s.concealedSamples; out.totalSamples = s.totalSamplesReceived;
      out.jitterMs = s.jitter != null ? s.jitter * 1000 : null;
      out.bytesReceived = s.bytesReceived; out.audioLevel = s.audioLevel;
    }
    if (s.type === 'outbound-rtp' && s.kind === 'audio') { out.packetsSent = s.packetsSent; out.bytesSent = s.bytesSent; }
    if (s.type === 'candidate-pair' && (s.nominated || s.selected) && s.state === 'succeeded' && s.currentRoundTripTime != null) {
      out.rttMs = s.currentRoundTripTime * 1000;
    }
  });
  if (S.mic) {
    const st = S.mic.getSettings();
    if (st.latency != null) out.inputLatencyMs = st.latency * 1000;
    out.micState = S.mic.readyState; out.micMuted = S.mic.muted;
  }
  return out;
}

function renderStats(s) {
  if (!s) return;
  const prev = S.lastStats;
  $('m-rtt').textContent = fmtMs(s.rttMs);
  $('m-jb').textContent = fmtMs(s.jbDelayMs);
  if (prev && s.packetsReceived != null && prev.packetsReceived != null) {
    const rec = s.packetsReceived - prev.packetsReceived, lost = (s.packetsLost || 0) - (prev.packetsLost || 0);
    $('m-loss').textContent = rec + lost > 0 ? `${((100 * Math.max(0, lost)) / (rec + lost)).toFixed(1)} %` : '0 %';
    const cs = (s.concealed || 0) - (prev.concealed || 0), ts = (s.totalSamples || 0) - (prev.totalSamples || 0);
    $('m-conceal').textContent = ts > 0 ? `${((100 * cs) / ts).toFixed(1)} %` : '—';
    $('m-rx').textContent = `${Math.round((8 * ((s.bytesReceived || 0) - (prev.bytesReceived || 0))) / 1000)} kbit/s`;
    $('m-tx').textContent = s.bytesSent != null ? `${Math.round((8 * (s.bytesSent - (prev.bytesSent || 0))) / 1000)} kbit/s` : '—';
  }
  S.lastStats = s;
}

function onPcStats(m) {
  S.pcStats = m;
  // El PC dice que no tiene sesión WebRTC pero el navegador cree que sí (p. ej.
  // la aplicación del PC se reinició): se renegocia sin esperar al timeout de ICE.
  const pcLost = S.running && m.stats && m.stats.net && !m.stats.net.connected && S.pc && S.pc.connectionState === 'connected';
  S.pcLostCount = pcLost ? (S.pcLostCount || 0) + 1 : 0;
  if (S.pcLostCount === 2) { logPC('warn', 'El PC no tiene sesión WebRTC: renegociando'); negotiate().catch((e) => showError(e.message)); }
  const r = m.stats && m.stats.render;
  if (r) {
    $('m-pcbuf').textContent = r.running ? `${r.buffer_ms.toFixed(0)} / ${r.target_ms.toFixed(0)} ms` : 'inactivo';
    $('m-pcund').textContent = String(r.underruns);
  }
  if (m.ca_fp) $('ca-fp').textContent = m.ca_fp;
  if (m.devices && S.running && mode() === 'call' && m.devices.mic_error) showError('PC: ' + m.devices.mic_error);
}

function sendReport() {
  send({ t: 'report', data: {
    stats: S.lastStats, mode: mode(), running: S.running,
    input: { label: S.inputLabel, class: S.inputClass, confirmed: $('confirm-route').checked },
    visibility: document.visibilityState, wakeLock: !!(S.wakeLock && !S.wakeLock.released),
    rtc: S.pc ? S.pc.connectionState : 'none',
  } });
}

// ---------- Ciclo de vida ----------
async function start() {
  showError('');
  if (!checkEnvironment()) return;
  const btn = $('btn-start'); btn.disabled = true;
  try {
    // Desbloqueo de la reproducción dentro del gesto del usuario (requisito de iOS).
    const el = $('remote'); el.srcObject = new MediaStream(); el.play().catch(() => {});

    if (mode() === 'call') {
      if (!S.mic || S.mic.readyState === 'ended') S.mic = await openMic($('input-select').value || null);
      if (!inputAllowed()) { S.mic.stop(); S.mic = null; return; }
    } else {
      if (S.mic) { S.mic.stop(); S.mic = null; }
      pill('st-input', 'sin micrófono (solo escucha)', '');
    }
    S.running = true; setRunningUI(true);
    keepAwake();
    if ('mediaSession' in navigator) {
      try { navigator.mediaSession.metadata = new MediaMetadata({ title: 'Virtual Audio Bridge', artist: 'Audio del PC' }); } catch (_) {}
    }
    await negotiate();
    clearInterval(S.statsTimer);
    S.statsTimer = setInterval(async () => { try { renderStats(await collectStats()); sendReport(); } catch (_) {} }, 1000);
  } catch (e) {
    const msgs = {
      NotAllowedError: 'Permiso de micrófono denegado. En iPhone: toca «aA» en la barra de direcciones → Ajustes del sitio web → Micrófono → Permitir.',
      NotFoundError: 'No hay ningún micrófono disponible. Conecta los auriculares Bluetooth al iPhone.',
      NotReadableError: 'El micrófono está ocupado por otra aplicación (p. ej. una llamada en curso).',
      OverconstrainedError: 'El micrófono elegido ya no está disponible. Vuelve a elegirlo.',
      SecurityError: 'El navegador bloquea el micrófono en esta página. Abre la dirección https:// del PC.',
    };
    showError(msgs[e.name] || ('No se pudo iniciar: ' + (e.message || e)));
    stop(false);
  } finally { btn.disabled = false; }
}

function stop(notify = true) {
  S.running = false; setRunningUI(false);
  clearInterval(S.statsTimer); clearTimeout(S.reconnectTimer);
  closePC();
  if (S.mic) { S.mic.stop(); S.mic = null; }
  if (S.wakeLock) { S.wakeLock.release().catch(() => {}); S.wakeLock = null; }
  $('remote').srcObject = null;
  pill('st-rtc', 'detenido', ''); pill('st-vis', '—', '');
  if (notify) send({ t: 'hangup' });
  sendReport();
}

$('btn-start').addEventListener('click', start);
$('btn-stop').addEventListener('click', () => stop(true));
$('btn-reconnect').addEventListener('click', async () => {
  showError('');
  const el = $('remote'); el.play().catch(() => {});
  if (mode() === 'call' && (!S.mic || S.mic.readyState === 'ended')) {
    try { S.mic = await openMic($('input-select').value || null); if (!inputAllowed()) return; }
    catch (e) { showError('No se pudo reabrir el micrófono: ' + e.message); return; }
  }
  negotiate().catch((e) => showError(e.message));
});
$('input-select').addEventListener('change', () => { if (S.mic) { S.mic.stop(); S.mic = null; } });
$('confirm-route').addEventListener('change', () => { if ($('confirm-route').checked) showError(''); });

(async () => {
  pill('st-rtc', 'detenido', ''); pill('st-input', '—', ''); pill('st-vis', '—', '');
  if (!checkEnvironment()) return;
  try { if (await pairIfNeeded()) connectWS(); }
  catch (e) { showError('No se puede contactar con el PC: ' + e.message); }
})();
