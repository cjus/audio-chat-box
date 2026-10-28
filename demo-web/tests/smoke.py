#!/usr/bin/env python3
"""HTTP/protocol integration using synthetic silence and a fake speech child; no model download."""
import base64
import http.client
import io
import json
import os
from pathlib import Path
import subprocess
import socket
import sys
import tempfile
import time
import wave

ROOT = Path(__file__).resolve().parents[1]
with io.BytesIO() as output:
    with wave.open(output, 'wb') as w:
        w.setnchannels(1); w.setsampwidth(2); w.setframerate(16000); w.writeframes(b'\0\0' * 160)
    WAV = output.getvalue()

def request(path, body=None, origin='http://localhost:8080', host='localhost:8080'):
    c = http.client.HTTPConnection('127.0.0.1', 8080, timeout=5)
    c.request('GET' if body is None else 'POST', path, body, {'Host': host, 'Origin': origin})
    r = c.getresponse()
    try: data = r.read()
    except ConnectionResetError:
        if r.status < 400: raise
        data = b'' # Rejected oversized/foreign body may remain unread at close.
    result = (r.status, data); c.close(); return result

with tempfile.TemporaryDirectory() as tmp:
    fake = Path(tmp) / 'fake-speech'
    fake.write_text('#!/usr/bin/env python3\n' + '''import sys,json,base64,time,os
for line in sys.stdin:
 v=json.loads(line); m=v['method']; i=v.get('id')
 if m=='initialize':
  print(json.dumps({'id':i,'result':{}}),flush=True)
  for model in ['kokoro','parakeet']:
   print(json.dumps({'method':'voice/modelState','params':{'id':model,'state':{'stage':'ready'}}}),flush=True)
 elif m=='voice/voices':
  print(json.dumps({'id':i,'result':{'voices':[{'id':'test-voice','name':'Test','language':'en-US'}]}}),flush=True)
 elif m=='voice/transcribe':
  print(json.dumps({'id':i,'result':{'text':'Synthetic test'}}),flush=True)
 elif m=='voice/speak':
  if v['params']['text']=='slow':
   with open(os.environ['FAKE_COUNTER'],'a') as f: f.write('job\\n')
   time.sleep(.8)
  assert v['params'].get('voice') in (None, 'test-voice')
  print(json.dumps({'id':i,'result':{'audio':''' + repr(base64.b64encode(WAV).decode()) + '''}}),flush=True)
''')
    fake.chmod(0o700)
    counter = Path(tmp) / 'count'
    env = dict(os.environ, FAKE_COUNTER=str(counter), AUDIO_CHAT_BOX_BIN=str(fake), DEMO_ORIGIN='http://localhost:8080')
    server = subprocess.Popen([str(ROOT/'target/debug/audio-chat-box-demo')], env=env)
    try:
        for _ in range(100):
            if server.poll() is not None: raise RuntimeError('Server exited')
            try:
                status, body = request('/api/status')
                if status == 200 and json.loads(body)['ready']: break
            except OSError: pass
            time.sleep(.05)
        else: raise RuntimeError('Server not ready')
        assert request('/')[0] == 200
        assert json.loads(request('/api/voices')[1])['voices'][0]['id'] == 'test-voice'
        assert request('/api/speak',b'{"text":"hello","voice":"test-voice"}') == (200,WAV)
        assert request('/api/speak',b'{"text":"hello","voice":"unknown"}')[0] == 400
        assert request('/api/transcribe', WAV) == (200,b'{"text":"Synthetic test"}')
        assert request('/api/speak',b'{"text":"hello"}') == (200,WAV)
        assert request('/api/speak',b'{"text":"hello"}',origin='https://evil.example')[0] == 400
        assert request('/api/status',host='evil.example')[0] == 400
        assert request('/api/transcribe',b'webm')[0] == 400
        assert request('/api/speak',json.dumps({'text':'x'*1001}).encode())[0] == 400
        oversized = socket.create_connection(('127.0.0.1',8080))
        oversized.sendall(b'POST /api/transcribe HTTP/1.1\r\nHost: localhost:8080\r\nOrigin: http://localhost:8080\r\nContent-Length: 960045\r\n\r\n')
        assert oversized.recv(1024).startswith(b'HTTP/1.1 400')
        oversized.close()
        # Abandon an HTTP request while its fake native inference continues. Repeated
        # new requests must remain busy rather than creating abandoned native jobs.
        sock = socket.create_connection(('127.0.0.1',8080))
        payload = b'{"text":"slow"}'
        sock.sendall(b'POST /api/speak HTTP/1.1\r\nHost: localhost:8080\r\nOrigin: http://localhost:8080\r\nContent-Length: '+str(len(payload)).encode()+b'\r\n\r\n'+payload)
        for _ in range(100):
            if counter.exists(): break
            time.sleep(.01)
        assert counter.exists()
        sock.close()
        for _ in range(8): assert request('/api/speak',b'{"text":"slow"}')[0] == 503
        assert counter.read_text().count('job') == 1
        time.sleep(.9)
        assert request('/api/speak',b'{"text":"hello"}') == (200,WAV)
        print('HTTP smoke passed: routes, voices, malformed/oversized/cross-origin input, and repeated-abort single-worker bound')
    finally:
        # SIGTERM of the demo may not run Rust Drop; terminate its test child explicitly.
        server.terminate(); server.wait(timeout=5)
