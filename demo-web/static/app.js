import { encodeWav, SAMPLE_RATE, MAX_SECONDS } from './recorder.js';

const $ = (id) => document.getElementById(id);
let ready = false;
let voicesReady = false;
let voicesLoading = false;
let mode = 'idle';
let generation = 0;
let capture = null;
let request = null;
let timer = null;
const urls = new Map();

function message(text) { $('status').textContent = text; }
function controls() {
  $('record').disabled = !ready || !voicesReady || mode !== 'idle';
  $('stop').disabled = mode !== 'recording';
  $('cancel').disabled = mode === 'idle';
  $('speak').disabled = !ready || !voicesReady || mode !== 'idle' || !$('text').value.trim();
  $('voice').disabled = !ready || !voicesReady || mode !== 'idle';
  $('text').disabled = mode !== 'idle';
}
function disposeCapture() {
  clearTimeout(timer);
  timer = null;
  const old = capture;
  capture = null;
  if (!old) return;
  old.stream?.getTracks().forEach((track) => track.stop());
  old.node?.disconnect();
  old.source?.disconnect();
  if (old.context && old.context.state !== 'closed') void old.context.close().catch(() => {});
}
function clearAudio(id) {
  const element = $(id);
  element.pause();
  element.removeAttribute('src');
  element.load();
  if (urls.has(id)) URL.revokeObjectURL(urls.get(id));
  urls.delete(id);
}
function setAudio(id, blob) {
  clearAudio(id);
  const url = URL.createObjectURL(blob);
  urls.set(id, url);
  $(id).src = url;
}
function cancel() {
  generation += 1;
  request?.abort();
  request = null;
  disposeCapture();
  mode = 'idle';
  $('recording').pause();
  $('playback').pause();
  message('Cancelled. The microphone is off.');
  controls();
}
async function checkedFetch(path, options, signal) {
  const response = await fetch(path, { ...options, signal, credentials: 'same-origin' });
  if (!response.ok) {
    // Error responses are displayed only as text, never injected as HTML.
    const detail = (await response.text()).slice(0, 300);
    throw new Error(`Service error ${response.status}: ${detail || response.statusText}`);
  }
  return response;
}
async function speakText(text, token, signal) {
  const response = await checkedFetch('/api/speak', {
    method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ text, voice: $('voice').value }),
  }, signal);
  const blob = await response.blob();
  if (generation !== token) return;
  setAudio('playback', new Blob([blob], { type: 'audio/wav' }));
  message('Ready to hear it back. Press Play on Kokoro playback.');
}
async function startRecording() {
  if (mode !== 'idle' || !ready || !voicesReady) return;
  if (!window.isSecureContext || !navigator.mediaDevices?.getUserMedia || !window.AudioWorkletNode) {
    message('Microphone recording needs a modern browser on localhost or HTTPS. You can still type text to read aloud.');
    return;
  }
  const token = ++generation;
  mode = 'starting'; controls();
  clearAudio('recording'); clearAudio('playback');
  message('Allow microphone access to begin. Cancel works while permission is pending.');
  const current = { chunks: [], stream: null, context: null, source: null, node: null };
  capture = current;
  try {
    const stream = await navigator.mediaDevices.getUserMedia({ audio: { channelCount: 1, echoCancellation: true }, video: false });
    if (generation !== token) { stream.getTracks().forEach((track) => track.stop()); return; }
    current.stream = stream;
    current.context = new AudioContext();
    await current.context.audioWorklet.addModule('/recorder.js');
    if (generation !== token) return;
    current.node = new AudioWorkletNode(current.context, 'demo-recorder', { channelCount: 1, channelCountMode: 'explicit' });
    current.node.port.onmessage = ({ data }) => {
      if (generation !== token || mode !== 'recording') return;
      if (data.samples) current.chunks.push(data.samples);
      if (data.ended) void stopRecording();
    };
    current.source = current.context.createMediaStreamSource(stream);
    current.source.connect(current.node);
    // The processor outputs silence: connecting it makes processing reliable without feedback.
    current.node.connect(current.context.destination);
    await current.context.resume();
    if (generation !== token) return;
    mode = 'recording'; controls();
    message('Recording… Stop when finished (30 seconds maximum).');
    timer = setTimeout(() => void stopRecording(), MAX_SECONDS * 1000);
  } catch (error) {
    if (generation === token) { disposeCapture(); mode = 'idle'; message(`Could not record: ${error.message}`); controls(); }
  }
}
async function stopRecording() {
  if (mode !== 'recording' || !capture) return;
  const token = generation;
  const current = capture;
  const sampleRate = current.context.sampleRate;
  // Stop devices immediately, before conversion or network work.
  disposeCapture();
  mode = 'processing'; controls(); message('Converting and transcribing… Microphone is off.');
  request = new AbortController();
  const signal = request.signal;
  try {
    const count = Math.min(current.chunks.reduce((sum, chunk) => sum + chunk.length, 0), sampleRate * MAX_SECONDS);
    if (!count) throw new Error('No audio was captured. Please try again.');
    const samples = new Float32Array(count);
    let at = 0;
    for (const chunk of current.chunks) {
      const part = chunk.subarray(0, count - at); samples.set(part, at); at += part.length;
      if (at === count) break;
    }
    current.chunks.length = 0;
    // Browser resampling filters before downsampling; do not just drop 48 kHz samples.
    const offline = new OfflineAudioContext(1, Math.max(1, Math.floor(count * SAMPLE_RATE / sampleRate)), SAMPLE_RATE);
    const buffer = offline.createBuffer(1, count, sampleRate); buffer.copyToChannel(samples, 0);
    const source = offline.createBufferSource(); source.buffer = buffer; source.connect(offline.destination); source.start();
    const rendered = await offline.startRendering();
    if (generation !== token) return;
    const wav = new Blob([encodeWav(rendered.getChannelData(0))], { type: 'audio/wav' });
    setAudio('recording', wav);
    const response = await checkedFetch('/api/transcribe', { method: 'POST', headers: { 'Content-Type': 'audio/wav' }, body: wav }, signal);
    const result = await response.json();
    if (generation !== token) return;
    if (typeof result.text !== 'string') throw new Error('The service returned an invalid transcript.');
    $('text').value = result.text.slice(0, 1000);
    if (!result.text.trim()) { message('No speech was recognized. You can play your recording or try again.'); return; }
    message(result.text.length > 1000 ? 'Transcript shortened to 1,000 characters for this demo. Creating speech…' : 'Transcript ready. Creating the spoken version…');
    await speakText($('text').value, token, signal);
  } catch (error) {
    if (generation === token) message(`Could not finish: ${error.message}`);
  } finally {
    if (generation === token) { mode = 'idle'; request = null; controls(); }
  }
}
$('record').addEventListener('click', () => void startRecording());
$('stop').addEventListener('click', () => void stopRecording());
$('cancel').addEventListener('click', cancel);
$('clear').addEventListener('click', () => {
  cancel(); $('text').value = ''; clearAudio('recording'); clearAudio('playback');
  message('Cleared. Record something or type text.'); controls();
});
$('text').addEventListener('input', controls);
$('speak').addEventListener('click', async () => {
  if (mode !== 'idle' || !ready || !voicesReady || !$('text').value.trim()) return;
  const token = ++generation; request = new AbortController(); mode = 'processing'; controls();
  clearAudio('playback'); message('Creating speech…');
  try { await speakText($('text').value, token, request.signal); }
  catch (error) { if (generation === token) message(`Could not speak: ${error.message}`); }
  finally { if (generation === token) { request = null; mode = 'idle'; controls(); } }
});
async function loadVoices() {
  if (voicesReady || voicesLoading || !ready) return;
  voicesLoading = true;
  try {
    const response = await checkedFetch('/api/voices', {}, AbortSignal.timeout(5000));
    const result = await response.json();
    if (!Array.isArray(result.voices) || !result.voices.length || result.voices.some((voice) => typeof voice.id !== 'string' || !voice.id)) {
      throw new Error('No valid voice choices were returned.');
    }
    const select = $('voice');
    const previous = select.value;
    select.replaceChildren();
    for (const voice of result.voices) {
      const option = document.createElement('option');
      option.value = voice.id;
      option.textContent = `${voice.name || voice.id}${voice.language ? ` — ${voice.language}` : ''}`;
      if (voice.description) option.title = voice.description;
      select.appendChild(option);
    }
    if (result.voices.some((voice) => voice.id === previous)) select.value = previous;
    voicesReady = true;
    $('voice-status').textContent = 'Choose a voice, then Read aloud. Changing voices keeps your transcript.';
  } catch (error) {
    $('voice-status').textContent = `Voice list unavailable: ${error.message} Retrying shortly.`;
  } finally { voicesLoading = false; controls(); }
}
let polling = false;
async function checkStatus() {
  if (polling) return;
  polling = true;
  try {
    const response = await checkedFetch('/api/status', {}, AbortSignal.timeout(5000));
    const wasReady = ready;
    const status = await response.json(); ready = status.ready === true;
    $('models').textContent = status.detail || (ready ? 'Ready.' : 'Not ready.');
    if (ready) void loadVoices();
    if (mode === 'idle' && wasReady !== ready) message(ready ? 'Ready. Record something or type text.' : 'Speech service is not ready. Check model status.');
  } catch { ready = false; $('models').textContent = 'Cannot reach the local speech service. Check the container logs.'; }
  finally { polling = false; controls(); }
}
window.addEventListener('pagehide', () => { cancel(); clearAudio('recording'); clearAudio('playback'); });
void checkStatus();
setInterval(() => { if (mode === 'idle') void checkStatus(); }, 5000);
