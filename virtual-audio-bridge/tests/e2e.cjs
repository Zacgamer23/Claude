// Prueba extremo a extremo AUTOMATIZADA (SIMULACIÓN DE HARDWARE).
//
// Arranca el motor compilado con `--features test-io` (tono sintético en lugar
// del altavoz de Windows y un medidor en lugar del micrófono virtual) y abre la
// página del móvil en Chromium con micrófono simulado. Comprueba:
//   1. emparejamiento con token de un solo uso,
//   2. señalización WSS + conexión WebRTC sin STUN/TURN,
//   3. audio PC → navegador (paquetes y nivel recibidos en el navegador),
//   4. audio navegador → PC (paquetes decodificados y nivel en el medidor),
//   5. reconexión automática tras cortar la conexión desde el PC,
//   6. rechazo de un token reutilizado.
// No prueba Bluetooth, iOS ni WASAPI: eso requiere el hardware real.
'use strict';
const { chromium } = require('playwright');
const { spawn } = require('child_process');
const fs = require('fs');
const os = require('os');
const path = require('path');

const ROOT = path.resolve(__dirname, '..');
const BIN = path.join(ROOT, 'engine/target/debug/vab-engine');
const IP = process.env.VAB_TEST_IP || '127.0.0.1';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function assert(cond, msg) { if (!cond) throw new Error('FALLO: ' + msg); console.log('  ✓ ' + msg); }

async function main() {
  const data = fs.mkdtempSync(path.join(os.tmpdir(), 'vab-e2e-'));
  fs.writeFileSync(path.join(data, 'config.json'), JSON.stringify({
    bind_ip: IP, https_port: 47900, udp_port: 47901, admin_port: 47902,
  }));
  let engLog = '', eng = null, token = '';
  const launch = async () => {
    eng = spawn(BIN, ['--no-browser', '--print-admin-url'], { env: { ...process.env, VAB_DATA_DIR: data } });
    eng.stderr.on('data', (d) => (engLog += d));
    const adminUrl = await new Promise((res, rej) => {
      eng.stdout.on('data', (d) => { const m = String(d).match(/http:\/\/127\.0\.0\.1:\d+\/#k=[0-9a-f]+/); if (m) res(m[0]); });
      eng.on('exit', (c) => rej(new Error('el motor terminó: ' + c + '\n' + engLog)));
      setTimeout(() => rej(new Error('el motor no arrancó\n' + engLog)), 15000);
    });
    token = adminUrl.split('#k=')[1];
  };
  const kill = async () => { const e = eng; const done = new Promise((r) => e.on('exit', r)); e.kill('SIGINT'); await done; };
  await launch();
  const api = async (p, body) => {
    const r = await fetch('http://127.0.0.1:47902/api/' + p, {
      method: body ? 'POST' : 'GET', headers: { 'X-Admin-Token': token, 'Content-Type': 'application/json' },
      body: body ? JSON.stringify(body) : undefined,
    });
    return r.json();
  };

  const browser = await chromium.launch({
    args: ['--use-fake-device-for-media-stream', '--use-fake-ui-for-media-stream', '--ignore-certificate-errors',
      '--autoplay-policy=no-user-gesture-required'],
  });
  let failed = false;
  try {
    console.log('Prueba E2E (simulación: tono sintético + micrófono falso de Chromium)');
    const st0 = await api('status');
    assert(st0.ok && st0.test_build, 'interfaz local responde (compilación de prueba)');
    assert((await (await fetch('http://127.0.0.1:47902/api/status')).json()).ok === false, 'la API local exige token');

    const ctx = await browser.newContext({ ignoreHTTPSErrors: true });
    const page = await ctx.newPage();
    page.on('console', (m) => { if (m.type() === 'error') console.log('    [navegador] ' + m.text()); });
    await page.goto(st0.pairing_url);
    await page.waitForFunction(() => document.getElementById('st-ws').textContent === 'conectado', null, { timeout: 10000 });
    assert(true, 'emparejado y señalización WSS conectada');
    assert(!page.url().includes('#t='), 'el token se elimina de la URL');

    // Primer intento: el micrófono falso no es identificable → debe pedir confirmación.
    await page.click('#btn-start');
    await page.waitForSelector('#confirm-box:not([hidden])', { timeout: 10000 });
    assert(true, 'micrófono no identificable: exige confirmación y no transmite');
    await page.check('#confirm-route');
    await page.click('#btn-start');
    await page.waitForFunction(() => document.getElementById('st-rtc').textContent === 'conectado', null, { timeout: 15000 });
    assert(true, 'WebRTC conectado sin STUN/TURN');

    await sleep(6000);
    const st = await api('status');
    const brow = await page.evaluate(() => S.lastStats);
    console.log('    navegador:', JSON.stringify(brow));
    console.log('    PC:', JSON.stringify({ net: st.stats.net, rx: st.stats.rx, render: st.stats.render, tx: st.stats.tx }));
    assert(st.stats.net.connected, 'el PC ve la conexión');
    assert(st.stats.tx.packets > 400, `PC→navegador: ${st.stats.tx.packets} paquetes enviados`);
    assert(brow.packetsReceived > 400, `navegador recibe ${brow.packetsReceived} paquetes`);
    assert(brow.audioLevel > 0.1, `navegador recibe nivel de audio ${brow.audioLevel.toFixed(2)} (tono de prueba)`);
    assert(st.stats.rx.packets > 200, `navegador→PC: ${st.stats.rx.packets} paquetes recibidos`);
    // El micrófono falso de Chromium emite pitidos cortos: se toma el máximo en 3 s.
    let peak = 0;
    for (let i = 0; i < 60; i++) { peak = Math.max(peak, (await api('status')).stats.render.peak); await sleep(50); }
    assert(peak > 0.01, `el medidor del PC recibe nivel ${peak.toFixed(2)} (pitidos del micrófono falso)`);
    assert(st.phone.report && st.phone.report.input && st.phone.report.input.confirmed, 'el PC recibe el informe del móvil');
    const results = {
      rtt_ms: st.stats.net.rtt_ms, browser_jitter_buffer_ms: brow.jbDelayMs,
      pc_mic_buffer_ms: st.stats.render.buffer_ms, pc_mic_target_ms: st.stats.render.target_ms,
      pc_mic_underruns: st.stats.render.underruns, plc_s: st.stats.rx.plc_frames / 48000,
    };

    const connected = () => page.waitForFunction(() => document.getElementById('st-rtc').textContent === 'conectado' &&
      S.pc && S.pc.connectionState === 'connected', null, { timeout: 45000, polling: 500 });

    // 1) Reinicio de la aplicación del PC: la página debe reconectarse sola
    //    (señalización y WebRTC) sin volver a escanear el QR.
    await kill();
    await page.waitForFunction(() => document.getElementById('st-ws').textContent !== 'conectado', null, { timeout: 10000 });
    await launch();
    let st2 = null;
    for (let i = 0; i < 90; i++) { st2 = await api('status'); if (st2.stats.net.connected && st2.stats.rx.packets > 50) break; await sleep(500); }
    await connected();
    assert(st2.stats.net.connected && st2.stats.rx.packets > 50, 'reconexión automática tras reiniciar la aplicación del PC (sin reemparejar)');

    // 2) «Desconectar móvil» desde el PC: la página se detiene y lo explica.
    await api('disconnect', {});
    await page.waitForFunction(() => /Desconectado desde el PC/.test(document.getElementById('error-text').textContent), null, { timeout: 10000 });
    assert(!(await page.evaluate(() => S.running)), 'desconexión desde el PC: la página se detiene y lo indica');

    // Token reutilizado: debe rechazarse.
    const r = await page.evaluate(async (u) => {
      const t = u.split('#t=')[1];
      const x = await fetch('/api/pair', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ token: t }) });
      return x.status;
    }, st0.pairing_url);
    assert(r === 403, 'un token ya usado se rechaza');

    fs.writeFileSync(path.join(ROOT, 'tests/e2e-results.json'), JSON.stringify({
      date: new Date().toISOString(), kind: 'SIMULACIÓN (Chromium Linux, loopback, tono sintético)', results,
    }, null, 2));
    console.log('    medidas (simulación, misma máquina):', JSON.stringify(results));
    console.log('TODAS LAS COMPROBACIONES SUPERADAS');
  } catch (e) {
    failed = true;
    console.error(e.message);
    console.error('--- registro del motor ---\n' + engLog.slice(-4000));
  } finally {
    await browser.close();
    if (eng && eng.exitCode === null) await kill();
    fs.rmSync(data, { recursive: true, force: true });
  }
  process.exit(failed ? 1 : 0);
}
main().catch((e) => { console.error(e); process.exit(1); });
