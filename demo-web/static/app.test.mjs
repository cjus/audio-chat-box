import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import vm from 'node:vm';
const source = (await readFile(new URL('./app.js', import.meta.url), 'utf8')).replace(/^import .*;\n/, '');
const voiceList = { voices: [{ id: 'test-a', name: 'Test A', language: 'en-US' }, { id: 'test-b', name: 'Test B', language: 'en-GB' }] };
const tick = () => new Promise((resolve) => setImmediate(resolve));
test('cancel pending microphone permission stops a late stream; repeated record does not prompt twice', async () => {
  const elements = new Map();
  function element(id) {
    if (!elements.has(id)) elements.set(id, {
      value: '', listeners: {}, textContent: '', disabled: false,
      addEventListener(name, callback) { this.listeners[name] = callback; },
      replaceChildren() { this.value = ''; }, appendChild(option) { if (!this.value) this.value = option.value; }, pause() {}, removeAttribute() {}, load() {},
    });
    return elements.get(id);
  }
  let resolvePermission;
  let prompts = 0;
  let stopped = 0;
  const requests = [];
  const context = vm.createContext({
    document: { getElementById: element, createElement: () => ({}) },
    window: { isSecureContext: true, AudioWorkletNode: true, addEventListener() {} },
    navigator: { mediaDevices: { getUserMedia() { prompts += 1; return new Promise((resolve) => { resolvePermission = resolve; }); } } },
    fetch: async (path) => { requests.push(path); return { ok: true, json: async () => path === '/api/voices' ? voiceList : ({ ready: true }) }; },
    AbortSignal, AbortController, setTimeout, clearTimeout, setInterval() {}, URL,
  });
  vm.runInContext(source, context);
  await tick();
  element('record').listeners.click();
  element('record').listeners.click();
  assert.equal(prompts, 1);
  element('cancel').listeners.click();
  resolvePermission({ getTracks: () => [{ stop() { stopped += 1; } }] });
  await tick();
  assert.equal(stopped, 1);
  assert.equal(element('record').disabled, false);
  assert.equal(element('cancel').disabled, true);
  assert.deepEqual(requests, ['/api/status', '/api/voices']);
  assert.match(element('status').textContent, /Cancelled/);
});
test('worklet bounds sample capture and sends one end event', async () => {
  let Processor;
  const messages = [];
  class Base { constructor() { this.port = { postMessage: (value) => messages.push(value) }; } }
  const worklet = (await readFile(new URL('./recorder.js', import.meta.url), 'utf8')).replaceAll('export ', '');
  vm.runInNewContext(worklet, { AudioWorkletProcessor: Base, sampleRate: 16000,
    registerProcessor: (_, value) => { Processor = value; }, Float32Array });
  const processor = new Processor();
  for (let i = 0; i < 4000; i += 1) processor.process([[new Float32Array(128)]]);
  assert.equal(messages.reduce((count, value) => count + (value.samples?.length || 0), 0), 480000);
  assert.equal(messages.filter((value) => value.ended).length, 1);
});
test('cancel ignores a late synthesized response and restores editable controls', async () => {
  const elements = new Map();
  function element(id) {
    if (!elements.has(id)) elements.set(id, { value: '', listeners: {}, textContent: '', disabled: false,
      addEventListener(name, callback) { this.listeners[name] = callback; }, replaceChildren() { this.value = ''; }, appendChild(option) { if (!this.value) this.value = option.value; }, pause() {}, removeAttribute() {}, load() {} });
    return elements.get(id);
  }
  let finishSpeak;
  let creates = 0;
  let spoken;
  const context = vm.createContext({
    document: { getElementById: element, createElement: () => ({}) }, window: { addEventListener() {} },
    fetch: async (path, options) => path === '/api/status' || path === '/api/voices'
      ? { ok: true, json: async () => path === '/api/voices' ? voiceList : ({ ready: true }) }
      : new Promise((resolve) => { spoken = JSON.parse(options.body); finishSpeak = () => resolve({ ok: true, blob: async () => new Blob(['fake']) }); }),
    AbortSignal, AbortController, Blob, setTimeout, clearTimeout, setInterval() {},
    URL: { createObjectURL() { creates += 1; return 'blob:test'; }, revokeObjectURL() {} },
  });
  vm.runInContext(source, context);
  await tick();
  element('text').value = 'Synthetic fixture';
  element('voice').value = 'test-b';
  element('speak').listeners.click();
  assert.equal(spoken.voice, 'test-b');
  assert.equal(spoken.text, 'Synthetic fixture');
  assert.equal(element('text').value, 'Synthetic fixture');
  assert.equal(element('voice').disabled, true);
  element('cancel').listeners.click();
  finishSpeak();
  await tick();
  assert.equal(creates, 0);
  assert.equal(element('text').disabled, false);
  assert.equal(element('speak').disabled, false);
  assert.equal(element('voice').disabled, false);
  assert.match(element('status').textContent, /Cancelled/);
});
