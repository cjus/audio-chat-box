import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
// No package metadata or browser globals are required to test the pure codec.
const code = await readFile(new URL('./recorder.js', import.meta.url), 'utf8');
const { encodeWav } = await import(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`);
test('mono PCM16 header and clipped little-endian samples', () => {
  const bytes = encodeWav(new Float32Array([-2, -1, 0, 1, 2, NaN]));
  const view = new DataView(bytes);
  assert.equal(Buffer.from(bytes).subarray(0, 4).toString(), 'RIFF');
  assert.equal(view.getUint32(4, true), bytes.byteLength - 8);
  assert.equal(view.getUint16(20, true), 1);
  assert.equal(view.getUint16(22, true), 1);
  assert.equal(view.getUint32(24, true), 16000);
  assert.equal(view.getUint32(28, true), 32000);
  assert.equal(view.getUint16(34, true), 16);
  assert.equal(view.getUint32(40, true), 12);
  assert.deepEqual(Array.from({ length: 6 }, (_, i) => view.getInt16(44 + i * 2, true)), [-32768, -32768, 0, 32767, 32767, 0]);
});
test('30 seconds stays within the server WAV limit', () => {
  assert.equal(encodeWav(new Float32Array(30 * 16000)).byteLength, 960044);
});
