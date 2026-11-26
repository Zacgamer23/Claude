// Interfaz local del PC: lee el estado real del motor cada segundo.
'use strict';
const $ = (id) => document.getElementById(id);

let token = '';
try {
  const m = location.hash.match(/k=([0-9a-f]{32})/);
  if (m) { token = m[1]; sessionStorage.setItem('vab_k', token); history.replaceState(null, '', '/'); }
  else token = sessionStorage.getItem('vab_k') || '';
} catch (_) {}

async function api(path, body) {
  const r = await fetch('/api/' + path, {
    method: body === undefined ? 'GET' : 'POST',
    headers: { 'X-Admin-Token': token, 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const j = await r.json().catch(() => ({ ok: false, error: 'respuesta no válida' }));
  if (!r.ok || !j.ok) throw new Error(j.error || r.statusText);
  return j;
}

function set(id, text, cls) { const el = $(id); el.textContent = text; el.className = cls || ''; }
const ms = (v) => (v == null || isNaN(v) ? '—' : `${Number(v).toFixed(0)} ms`);
let last = null, lastPc = null, bridgeOn = true, cfgLoaded = false;

function render(s) {
  const st = s.stats, ph = s.phone || {}, rep = ph.report || {}, pst = rep.stats || {};
  $('sub').textContent = `Escuchando en ${s.origin} · activo ${Math.floor(s.uptime_s / 60)} min${s.test_build ? ' · COMPILACIÓN DE PRUEBA' : ''}`;
  bridgeOn = s.bridge;
  $('btn-bridge').textContent = s.bridge ? 'Detener puente' : 'Iniciar puente';
  if ($('qr').dataset.url !== s.pairing_url) { $('qr').innerHTML = s.qr_svg; $('qr').dataset.url = s.pairing_url; }
  $('url').textContent = s.origin;
  $('expires').textContent = `${Math.floor(s.pairing_expires_s / 60)}:${String(s.pairing_expires_s % 60).padStart(2, '0')}`;
  $('fp-ca').textContent = s.ca_fingerprint; $('fp-srv').textContent = s.cert_fingerprint;

  set('s-ws', ph.ws_connected ? 'conectado' : 'sin conexión', ph.ws_connected ? 'ok' : 'warn');
  set('s-rtc', st.net.connected ? 'conectado' : (rep.rtc && rep.rtc !== 'none' ? rep.rtc : 'sin conexión'), st.net.connected ? 'ok' : 'warn');
  const inp = rep.input || {};
  const icls = { bluetooth: 'ok', builtin: 'bad', unknown: 'warn' }[inp.class] || '';
  set('s-input', rep.mode === 'listen' ? 'solo escucha (sin micrófono)' :
    inp.label ? `${inp.label}${inp.class === 'unknown' ? (inp.confirmed ? ' (confirmada por el usuario)' : ' (sin verificar)') : ''}` : '—', icls);
  set('s-vis', rep.visibility ? (rep.visibility === 'visible' ? 'en primer plano' : 'en segundo plano / bloqueada') + (rep.wakeLock ? ', pantalla retenida' : '') : '—',
    rep.visibility === 'visible' ? 'ok' : rep.visibility ? 'bad' : '');
  const d = s.devices || {};
  set('s-spk', d.speaker ? `${d.speaker.name} (${d.speaker.state})${st.capture.running ? '' : ' — detenido'}` : (d.speaker_error || '—'),
    d.speaker && st.capture.running ? 'ok' : 'bad');
  set('s-mic', d.mic_sink ? `${d.mic_sink.name} (${d.mic_sink.state})${st.render.running ? '' : ' — detenido'}` : (d.mic_error || '—'),
    d.mic_sink && st.render.running ? 'ok' : 'bad');

  set('l-rtt', st.net.connected ? ms(st.net.rtt_ms) : '—');
  set('l-jb', ms(pst.jbDelayMs));
  set('l-mic', st.render.running ? `${ms(st.render.buffer_ms)} / ${ms(st.render.target_ms)}` : '—');
  set('l-jit', ms(st.rx.jitter_ms));
  set('l-drift', `${st.render.drift_ppm} ppm`);
  set('l-fspk', st.capture.rate ? `${st.capture.rate} Hz, periodo ${(st.capture.period_us / 1000).toFixed(1)} ms → Opus 48 kHz` : '—');
  set('l-fmic', st.render.rate ? `Opus 48 kHz → ${st.render.rate} Hz, periodo ${(st.render.period_us / 1000).toFixed(1)} ms` : '—');

  set('q-loss', st.net.connected ? `${st.net.egress_loss_pct.toFixed(1)} % / ${st.net.ingress_loss_pct.toFixed(1)} %` : '—');
  set('q-plc', `${(st.rx.plc_frames / 48000).toFixed(2)} s`);
  set('q-mic', `${st.render.underruns} / ${st.render.overruns}`, st.render.underruns > (last ? last.stats.render.underruns : 0) ? 'warn' : '');
  set('q-cap', String(st.capture.overruns));
  set('q-pk', `${st.tx.packets} / ${st.rx.packets}`);
  $('q-txl').value = st.capture.peak; $('q-rxl').value = st.render.peak;

  const errs = [];
  if (d.speaker_error) errs.push('Altavoz: ' + d.speaker_error);
  if (d.mic_error) errs.push('Micrófono: ' + d.mic_error);
  $('banner').hidden = errs.length === 0; $('banner').textContent = errs.join(' · ');

  const logEl = $('log');
  const atBottom = logEl.scrollTop + logEl.clientHeight >= logEl.scrollHeight - 4;
  logEl.innerHTML = '';
  for (const e of s.log) {
    const line = document.createElement('div');
    line.className = e.level === 'error' ? 'e' : e.level === 'warn' ? 'w' : '';
    line.textContent = `${new Date(e.ts_ms).toLocaleTimeString()}  ${e.msg}`;
    logEl.appendChild(line);
  }
  if (atBottom) logEl.scrollTop = logEl.scrollHeight;

  if (!cfgLoaded) loadConfig(s.config);
  last = s;
}

async function loadConfig(cfg) {
  cfgLoaded = true;
  const f = $('cfg');
  for (const k of ['opus_bitrate', 'opus_frame_ms', 'jitter_min_ms', 'jitter_max_ms']) f.elements[k].value = cfg[k];
  let list = [];
  try { list = (await api('devices')).devices; } catch (e) { $('cfg-msg').textContent = e.message; }
  const fill = (sel, current, allowDefault) => {
    sel.innerHTML = '';
    if (allowDefault) { const o = document.createElement('option'); o.value = ''; o.textContent = '(salida predeterminada de Windows)'; sel.appendChild(o); }
    let found = !current;
    for (const d of list) {
      const o = document.createElement('option'); o.value = d.name; o.textContent = `${d.name} [${d.state}]${d.is_default ? ' ★' : ''}`;
      if (current && d.name.toLowerCase().includes(current.toLowerCase()) && !found) { o.selected = true; found = true; }
      sel.appendChild(o);
    }
    if (!found) { const o = document.createElement('option'); o.value = current; o.textContent = `${current} (no encontrado)`; o.selected = true; sel.appendChild(o); }
  };
  fill($('sel-spk'), cfg.speaker_device, true);
  fill($('sel-mic'), cfg.mic_sink_device, false);
}

$('cfg').addEventListener('submit', async (ev) => {
  ev.preventDefault();
  const f = ev.target.elements;
  try {
    const r = await api('config', {
      speaker_device: f.speaker_device.value, mic_sink_device: f.mic_sink_device.value,
      opus_bitrate: Number(f.opus_bitrate.value), opus_frame_ms: Number(f.opus_frame_ms.value),
      jitter_min_ms: Number(f.jitter_min_ms.value), jitter_max_ms: Number(f.jitter_max_ms.value),
    });
    $('cfg-msg').textContent = 'Guardado. ' + r.note;
  } catch (e) { $('cfg-msg').textContent = 'Error: ' + e.message; }
});
$('btn-bridge').addEventListener('click', () => api('bridge', { enabled: !bridgeOn }).catch(alert));
$('btn-disconnect').addEventListener('click', () => api('disconnect', {}).catch(alert));
$('btn-newcode').addEventListener('click', () => api('new-pairing', {}).catch(alert));
$('btn-revoke').addEventListener('click', () => {
  if (confirm('¿Revocar todos los móviles emparejados? Tendrán que volver a escanear el código.')) api('revoke', {}).catch(alert);
});
$('btn-quit').addEventListener('click', () => {
  if (confirm('¿Cerrar Virtual Audio Bridge? El audio dejará de llegar al móvil y el micrófono virtual quedará en silencio.')) {
    api('quit', {}).then(() => { document.body.innerHTML = '<p style="padding:24px">Virtual Audio Bridge se ha cerrado. Puedes volver a abrirlo desde el menú Inicio.</p>'; }).catch(alert);
  }
});
$('btn-copy').addEventListener('click', () => { if (last) navigator.clipboard.writeText(last.origin).catch(() => {}); });

async function poll() {
  try { render(await api('status')); }
  catch (e) {
    $('sub').textContent = token ? 'Sin conexión con el motor: ' + e.message : 'Falta el token: abre la interfaz desde la aplicación o el acceso directo.';
  }
  setTimeout(poll, 1000);
}
poll();
