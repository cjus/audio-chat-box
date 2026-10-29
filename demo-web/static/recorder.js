// Pure WAV conversion is also imported by the UI and tested in Node. The worklet
// copies mono input; it never sends microphone samples outside this AudioContext.
export const SAMPLE_RATE = 16000;
export const MAX_SECONDS = 30;
export function encodeWav(samples) {
  const bytes = new ArrayBuffer(44 + samples.length * 2);
  const view = new DataView(bytes);
  const text = (at, value) => [...value].forEach((c, i) => view.setUint8(at + i, c.charCodeAt(0)));
  text(0, 'RIFF'); view.setUint32(4, bytes.byteLength - 8, true); text(8, 'WAVE');
  text(12, 'fmt '); view.setUint32(16, 16, true); view.setUint16(20, 1, true);
  view.setUint16(22, 1, true); view.setUint32(24, SAMPLE_RATE, true);
  view.setUint32(28, SAMPLE_RATE * 2, true); view.setUint16(32, 2, true); view.setUint16(34, 16, true);
  text(36, 'data'); view.setUint32(40, samples.length * 2, true);
  samples.forEach((sample, i) => {
    const clamped = Number.isFinite(sample) ? Math.max(-1, Math.min(1, sample)) : 0;
    view.setInt16(44 + i * 2, Math.round(clamped * (clamped < 0 ? 32768 : 32767)), true);
  });
  return bytes;
}
if (typeof AudioWorkletProcessor !== 'undefined') {
  class Recorder extends AudioWorkletProcessor {
    constructor() { super(); this.remaining = sampleRate * MAX_SECONDS; }
    process(inputs) {
      const input = inputs[0]?.[0];
      if (!input || this.remaining <= 0) return this.remaining > 0;
      const chunk = input.slice(0, Math.min(input.length, this.remaining));
      this.remaining -= chunk.length;
      this.port.postMessage({ samples: chunk }, [chunk.buffer]);
      if (!this.remaining) this.port.postMessage({ ended: true });
      return this.remaining > 0;
    }
  }
  registerProcessor('demo-recorder', Recorder);
}
