// Run after `cargo build --manifest-path demo-web/Cargo.toml` with Playwright installed:
// node demo-web/tests/browser.cjs
// Uses Chromium's synthetic device only. No real microphone, model or speech API.
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawn } = require('node:child_process');

(async () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'audio-demo-browser-'));
  const fake = path.join(tmp, 'fake-speech');
  fs.writeFileSync(fake, `#!/usr/bin/env python3
import sys,json,base64,io,wave,math,struct
b=io.BytesIO()
with wave.open(b,'wb') as w:
 w.setnchannels(1);w.setsampwidth(2);w.setframerate(16000)
 w.writeframes(b''.join(struct.pack('<h',int(3000*math.sin(i*2*math.pi*440/16000))) for i in range(4000)))
for line in sys.stdin:
 v=json.loads(line);m=v['method'];i=v.get('id')
 if m=='initialize':
  print(json.dumps({'id':i,'result':{}}),flush=True)
  for model in ['kokoro','parakeet']:
   print(json.dumps({'method':'voice/modelState','params':{'id':model,'state':{'stage':'ready'}}}),flush=True)
 elif m=='voice/voices':
  print(json.dumps({'id':i,'result':{'voices':[{'id':'test-a','name':'Test A','language':'en-US'},{'id':'test-b','name':'Test B','language':'en-GB'}]}}),flush=True)
 elif m=='voice/transcribe':
  print(json.dumps({'id':i,'result':{'text':'Synthetic browser speech'}}),flush=True)
 elif m=='voice/speak':
  print(json.dumps({'id':i,'result':{'audio':base64.b64encode(b.getvalue()).decode()}}),flush=True)
`, { mode: 0o700 });
  const server = spawn(path.resolve(__dirname, '../target/debug/audio-chat-box-demo'), [], {
    env: { ...process.env, AUDIO_CHAT_BOX_BIN: fake, DEMO_ORIGIN: 'http://localhost:8080' }, stdio: 'inherit',
  });
  let browser;
  try {
    for (let i = 0; i < 100; i++) {
      try { const status = await fetch('http://localhost:8080/api/status'); if ((await status.json()).ready) break; } catch {}
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || '/usr/bin/chromium', headless: true,
      args: ['--no-sandbox', '--use-fake-device-for-media-stream', '--use-fake-ui-for-media-stream'] });
    const page = await browser.newPage({ viewport: { width: 1000, height: 1000 } });
    const errors = [];
    const transcriptions = [];
    page.on('pageerror', (error) => errors.push(error.message));
    page.on('request', (request) => {
      if (request.url().endsWith('/api/transcribe')) transcriptions.push(request.postDataBuffer());
    });
    await page.goto('http://localhost:8080');
    await page.waitForFunction(() => !document.getElementById('record').disabled);
    assert.match(await page.title(), /audio-chat-box/);
    await page.locator('#text').fill('Synthetic typed fixture');
    await page.locator('#voice').selectOption('test-b');
    assert.equal(await page.locator('#text').inputValue(), 'Synthetic typed fixture');
    const speakRequest = page.waitForRequest((r) => r.url().endsWith('/api/speak'));
    await page.locator('#speak').click();
    assert.equal((await speakRequest).postDataJSON().voice, 'test-b');
    await page.waitForFunction(() => document.getElementById('playback').readyState >= 2);
    await page.locator('#playback').evaluate((audio) => audio.play());
    await page.waitForFunction(() => document.getElementById('playback').ended);
    assert.ok(await page.locator('#playback').evaluate((audio) => audio.duration > 0));
    await page.locator('#record').click();
    await page.waitForFunction(() => !document.getElementById('stop').disabled);
    await page.waitForTimeout(400);
    await page.locator('#stop').click();
    await page.waitForFunction(() => document.getElementById('text').value === 'Synthetic browser speech' && !document.getElementById('record').disabled);
    assert.equal(transcriptions.length, 1);
    assert.equal(transcriptions[0].readUInt32LE(24), 16000);
    assert.equal(transcriptions[0].readUInt16LE(22), 1);
    assert.equal(transcriptions[0].readUInt16LE(34), 16);
    assert.ok(transcriptions[0].length > 44 && transcriptions[0].length <= 960044);
    await page.waitForFunction(() => document.getElementById('recording').readyState >= 2);
    // Cancel a subsequent recording and confirm no request or stale microphone remains.
    await page.locator('#record').click();
    await page.waitForFunction(() => !document.getElementById('stop').disabled);
    await page.locator('#cancel').click();
    await page.waitForTimeout(100);
    assert.equal(transcriptions.length, 1);
    assert.equal(await page.locator('#record').isEnabled(), true);
    await page.locator('#clear').click();
    assert.equal(await page.locator('#text').inputValue(), '');
    assert.equal(await page.locator('#playback').getAttribute('src'), null);
    // Verify actual browser filtering/conversion from both common device rates.
    const resampling = await page.evaluate(async () => {
      const { encodeWav } = await import('/recorder.js');
      const results = [];
      for (const rate of [44100, 48000]) {
        const context = new OfflineAudioContext(1, 16000, 16000);
        const buffer = context.createBuffer(1, rate, rate);
        const input = buffer.getChannelData(0);
        for (let i = 0; i < rate; i++) input[i] = Math.sin(2 * Math.PI * 440 * i / rate) * .5;
        const source = context.createBufferSource(); source.buffer = buffer; source.connect(context.destination); source.start();
        const rendered = await context.startRendering();
        results.push({ frames: rendered.length, bytes: encodeWav(rendered.getChannelData(0)).byteLength,
          peak: Math.max(...rendered.getChannelData(0)) });
      }
      return results;
    });
    for (const result of resampling) { assert.equal(result.frames, 16000); assert.equal(result.bytes, 32044); assert.ok(result.peak > .4 && result.peak < .6); }
    assert.deepEqual(errors, []);
    console.log('Browser smoke passed: rendered page; synthetic typed playback; fake-mic transcription/readback; cancel/clear; 44.1/48 kHz resampling. No real models or microphone.');
  } finally {
    if (browser) await browser.close();
    server.kill('SIGTERM');
    await new Promise((resolve) => server.once('exit', resolve));
    fs.rmSync(tmp, { recursive: true, force: true });
  }
})().catch((error) => { console.error(error); process.exitCode = 1; });
