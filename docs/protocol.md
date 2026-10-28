# The audio-chat-box protocol

How a program talks to audio-chat-box. This page is all a client needs: the connection, the
messages, starting and stopping, the errors, and every voice method, with what
audio-chat-box itself does where the protocol leaves a choice.

It is the voice domain of the [Chess Node Protocol](https://github.com/cjus/chess-node-protocol)
(CNP 1.0, its sections 3 to 6, 8 and 9), which audio-chat-box grew up with (README,
"Origins"). Nothing here is about chess: a client that speaks CNP's voice domain talks to
audio-chat-box unchanged, and a client that knows only this page needs nothing else of CNP.
Adapted from CNP's specification, © 2026 Carlos Justiniano, under
[CC-BY-4.0](https://creativecommons.org/licenses/by/4.0/).

The words MUST, MUST NOT, SHOULD and MAY are used as in
[RFC 2119](https://www.rfc-editor.org/rfc/rfc2119). In the examples, `→` is a message from the
client and `←` one from audio-chat-box (the **node**); the arrows are not part of the messages.

## Contents

1. [The connection](#1-the-connection)
2. [Messages](#2-messages)
3. [Starting and stopping](#3-starting-and-stopping)
4. [What it can do](#4-what-it-can-do)
5. [Speaking and hearing](#5-speaking-and-hearing)
6. [Listening](#6-listening)
7. [Errors](#7-errors)
8. [Versions](#8-versions)

## 1. The connection

The client starts `audio-chat-box` as a child process (README, "Running it") and talks to it
through its standard input and output.

- Each message is one line of UTF-8 JSON, ending in `\n` (a `\r` before it is ignored). JSON
  escapes line breaks inside strings, so a message never spans lines. Blank lines are ignored.
- The node writes nothing else to its standard output. Its log goes to standard error.
- When its standard input ends, the client has gone, and the node ends as if it had received
  `exit`.
- A line longer than 16 MiB (`maxMessageBytes`) is refused with `InvalidRequest`, and the
  node reads on from the next line.

Field names are camelCase, and durations whole milliseconds in fields ending in `Ms`. Unknown
fields are ignored, so later versions can add some; `null` means the same as leaving a field
out. Audio travels as base64 ([RFC 4648](https://www.rfc-editor.org/rfc/rfc4648#section-4),
with padding).

## 2. Messages

Every message is a [JSON-RPC 2.0](https://www.jsonrpc.org/specification) message:

- A **request** has an `id` (a whole number or a string) and a `method`, and MAY have
  `params`, an object. It is answered by exactly one response with the same `id`. Don't reuse
  an `id` while its request is unanswered.
- A **notification** has a `method` and no `id`, and is never answered.
- A **response** has the request's `id` and either a `result` or an `error`
  ([7](#7-errors)). `"result": null` is a result.

Only the client sends requests; the node sends responses and the notifications below.
Responses may come in any order: the node works on several requests at once. Batches (arrays
of messages) aren't accepted. A method the node doesn't know is answered with
`MethodNotFound`, and a notification it doesn't know is ignored.

**Cancelling.** `$/cancelRequest` drops a request the node hasn't answered yet: the node
answers it at once with `Cancelled`, and nothing it did counts. Cancelling one already
answered, or one that never was, does nothing: the two messages can cross.

```json
→ {"jsonrpc": "2.0", "method": "$/cancelRequest", "params": {"id": 7}}
← {"jsonrpc": "2.0", "id": 7, "error": {"code": -32800, "message": "cancelled"}}
```

## 3. Starting and stopping

```text
  client                                       node
  initialize {protocol, client} ────────────▶
                ◀────────────── {protocol, node, capabilities, limits}
  … requests and notifications, ping at any time …
  shutdown ─────────────────────────────────▶  ends its work
                ◀────────────── null
  exit ─────────────────────────────────────▶  ends, with exit code 0
```

**`initialize`** is the first request. Before it, every other request is answered with
`NotInitialized`. Its params: `protocol`, the newest version the client speaks (`"1.0"`), and
`client`, with its `name` and `version`. The result: `protocol`, the version both speak from
then on; `node`, with `name` (`audio-chat-box`), `version`, `license` and `source`;
`capabilities` ([4](#4-what-it-can-do)); and `limits`. A client whose major version the node
doesn't speak is answered `IncompatibleVersion`, with the versions it does speak in `data`.

Right after it, the node starts getting its models ready, and says how each is getting on
(`voice/modelState`, [5.4](#54-voicemodels-and-voicemodelstate)).

**`ping`** takes no params and is answered with `null`, at any time: a busy node answers it
too.

**`shutdown`** takes no params: the node drops its running work as if each request had been
cancelled, and answers `null`; after it, requests are answered `ShuttingDown`. Then the
notification **`exit`** ends it, with exit code 0 (1 if `shutdown` didn't come first).

```json
→ {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocol": "1.0", "client": {"name": "my-app", "version": "1.0"}}}
← {"jsonrpc": "2.0", "id": 1, "result": {"protocol": "1.0", "node": {"name": "audio-chat-box", "version": "0.1.0", "license": "GPL-3.0-or-later", "source": "https://github.com/cjus/audio-chat-box"}, "capabilities": {"voice": {"speak": true, "transcribe": true, "listen": true, "hotwords": true, "formats": ["wav", "pcmF32"]}}, "limits": {}}}
→ {"jsonrpc": "2.0", "id": 99, "method": "shutdown"}
← {"jsonrpc": "2.0", "id": 99, "result": null}
→ {"jsonrpc": "2.0", "method": "exit"}
```

## 4. What it can do

`capabilities.voice` says what this node offers; a missing capability is `false`, and a
request that needs one it didn't offer is answered `Unsupported`.

| Capability | | audio-chat-box |
| --- | --- | --- |
| `speak` | `voice/speak` | With `--speak` (the default) |
| `transcribe` | `voice/transcribe` | With `--hear` (the default) |
| `listen` | `voice/listen` and its notifications | With `--listen silero-vad` |
| `hotwords` | `hotwords` in `voice/transcribe` and `voice/listen` | Whenever it hears |
| `wordTimes` | Each word's start and end | Never |
| `formats` | The audio formats it reads and writes | `wav` and `pcmF32` |

Audio formats: `wav` is a RIFF WAVE file, mono, with 16-bit integer or 32-bit float samples;
`pcmF32` is raw mono samples, 32-bit float, little-endian, at a stated sample rate.

**Hotwords.** A client that knows what is likely to be said gives it as `hotwords`: words
and phrases, which the node favours where the sound allows them ("knight" over "night" for a
chess board, a product's own names for another). This is how the node stays out of the
client's domain. audio-chat-box gives each piece of a hotword a small bonus in its beam
search (`HOTWORD_SCORE`, `src/hear.rs`), which pulls ordinary words towards hotwords only
when the sound allows both. A hotword that is empty or holds a line break, or more than 1000
of them, is `InvalidParams`; "/" and ":" inside one are read as spaces.

## 5. Speaking and hearing

The node downloads, checks and loads its own models (README, "The models"). A request whose
params are wrong is `InvalidParams` whether or not the models are ready; one whose model
isn't ready yet is then `NotReady`. audio-chat-box hears English only: another `language` is
`Unsupported`.

### 5.1 `voice/voices`

No params. The result: `voices`, each with `id`, `name`, `language` (a
[BCP 47](https://www.rfc-editor.org/info/bcp47) tag like `en-US`) and optionally
`description`. The first is the default. audio-chat-box lists Kokoro's 54 voices.

### 5.2 `voice/speak`

Text to speech. Params:

| Field | Type | |
| --- | --- | --- |
| `text` | string | Plain words, no markup |
| `voice` | string | A voice's id. Missing: the default |
| `speed` | number | 1.0 is the voice's own pace |
| `format` | string | The audio format wanted. Missing: `wav` |

An unknown voice is `InvalidParams`. The result: `format`, `sampleRate`, `audio` (base64)
and `durationMs`.

```json
→ {"jsonrpc": "2.0", "id": 20, "method": "voice/speak", "params": {"text": "Your table is ready.", "voice": "am_michael"}}
← {"jsonrpc": "2.0", "id": 20, "result": {"format": "wav", "sampleRate": 24000, "audio": "UklGR…", "durationMs": 1150}}
```

### 5.3 `voice/transcribe`

Speech to text, of a recording. Params:

| Field | Type | |
| --- | --- | --- |
| `audio` | string | The speech, base64 |
| `format` | string | Its format |
| `sampleRate` | integer | For `pcmF32` (a WAV file says its own) |
| `language` | string | The language spoken, a BCP 47 tag. Missing: English |
| `hotwords` | array of strings | Words and phrases likely to be said ([4](#4-what-it-can-do)). Missing: none |

Audio that can't be decoded is `InvalidParams`. The result: `text`, empty if nothing was
said.

```json
→ {"jsonrpc": "2.0", "id": 21, "method": "voice/transcribe", "params": {"audio": "UklGR…", "format": "wav", "hotwords": ["espresso", "oat milk"]}}
← {"jsonrpc": "2.0", "id": 21, "result": {"text": "An espresso with oat milk."}}
```

### 5.4 `voice/models` and `voice/modelState`

`voice/models` takes no params. Its result is `models`, each with `id`, `name`, `purpose`
(`speak`, `transcribe`, or `listen` for one that hears when speech starts and ends),
`license`, and `state`. A state has a `stage`, one of `waiting`, `downloading`, `unpacking`,
`loading`, `ready` and `failed`, in that order. While downloading it has `bytes` so far and
`of` in all; when failed, a `message` for people.

`voice/modelState` is a notification from the node, `{"id": …, "state": …}`, sent at each
new stage, and while downloading about once a second: a client can show a first download's
progress.

```json
← {"jsonrpc": "2.0", "method": "voice/modelState", "params": {"id": "kokoro", "state": {"stage": "downloading", "bytes": 52000000, "of": 349906910}}}
← {"jsonrpc": "2.0", "method": "voice/modelState", "params": {"id": "kokoro", "state": {"stage": "ready"}}}
```

## 6. Listening

**`voice/listen`** (capability `listen`) hears speech as it comes, so that someone can talk
to the client without pressing anything. The client streams what its microphone hears; the
node says when someone starts speaking and, once they pause, what they said. A listen runs
until its audio ends or it is cancelled. audio-chat-box hears one listen at a time, and
answers a second with `Busy`. Params:

| Field | Type | |
| --- | --- | --- |
| `format` | string | `pcmF32`, the format that streams |
| `sampleRate` | integer | 16000, the only rate audio-chat-box listens to |
| `language` | string | The language spoken. Missing: English |
| `endSilenceMs` | integer | How long a pause ends what someone is saying. Missing: 800 |
| `minSpeechMs` | integer | Shorter sounds aren't speech (a cough, a click). Missing: 150 |
| `maxSpeechMs` | integer | The longest one utterance may be: at this length it ends, pause or not. Missing: 30000 |
| `leadInMs` | integer | How much of the audio from before an utterance's start it is heard with, so that a short first word said fast isn't cut off; 0 for none. Missing: 500 |
| `hotwords` | array of strings | Words and phrases likely to be said, in every utterance ([4](#4-what-it-can-do)). Missing: none |

Wrong params are `InvalidParams`: a duration of 0 (but `leadInMs`), or over 10000 (60000 for
`maxSpeechMs`), included.

The audio comes in **`voice/audio`** notifications from the client, in order: `id`, the
listen's; `audio`, the next samples, base64; and `end: true` when no more will come (`audio`
may then be left out). The node ignores audio for a listen it has already answered, since
the two can cross.

Times are milliseconds into the listen's audio, from its first sample. For each utterance,
the node sends:

- **`voice/speechStarted`**, `{"id": …, "startMs": …}`, once it is sure someone is speaking
  (after `minSpeechMs` of speech). `startMs` is about where they started.
- **`voice/heard`**, once they have paused for `endSilenceMs` and the node has heard them:
  `id`, `text` (empty if no words were made out), `startMs` and `endMs`.

Each `voice/speechStarted` is followed by one `voice/heard`, before the next utterance's
`voice/speechStarted`. When the audio ends, the node ends the utterance it is hearing, if
any, sends its `voice/heard`, and answers the listen with `durationMs`, how much audio it
heard in all. A cancelled listen drops what it was hearing: no `voice/heard` follows.

The audio is shortened here:

```json
→ {"jsonrpc": "2.0", "id": 30, "method": "voice/listen", "params": {"format": "pcmF32", "sampleRate": 16000, "hotwords": ["espresso", "latte"]}}
→ {"jsonrpc": "2.0", "method": "voice/audio", "params": {"id": 30, "audio": "AAAAAAAAAAA="}}
← {"jsonrpc": "2.0", "method": "voice/speechStarted", "params": {"id": 30, "startMs": 1240}}
→ {"jsonrpc": "2.0", "method": "voice/audio", "params": {"id": 30, "audio": "AAAAAAAAAAA="}}
← {"jsonrpc": "2.0", "method": "voice/heard", "params": {"id": 30, "text": "A latte, please.", "startMs": 1210, "endMs": 2380}}
→ {"jsonrpc": "2.0", "method": "voice/audio", "params": {"id": 30, "end": true}}
← {"jsonrpc": "2.0", "id": 30, "result": {"durationMs": 4000}}
```

## 7. Errors

An error has a `code`, a `message` for people, and optionally `data`. The codes
audio-chat-box answers with:

| Code | Name | When |
| --- | --- | --- |
| -32700 | `ParseError` | The line isn't JSON. The `id` is `null` |
| -32600 | `InvalidRequest` | The JSON isn't a valid message, or the request isn't allowed now (a second `initialize`, a batch, a line too long) |
| -32601 | `MethodNotFound` | An unknown method |
| -32602 | `InvalidParams` | Params missing, of the wrong shape, or with a wrong value |
| -32603 | `InternalError` | Something went wrong inside the node |
| -32002 | `NotInitialized` | A request before `initialize` |
| -32800 | `Cancelled` | The request was cancelled ([2](#2-messages)) |
| -32010 | `IncompatibleVersion` | The client's major version isn't spoken. `data`: `{"supported": [...]}` |
| -32013 | `Busy` | A second listen while one runs |
| -32014 | `Unsupported` | A capability it didn't offer, or a language it doesn't hear |
| -32016 | `ShuttingDown` | A request after `shutdown` |
| -32017 | `NotReady` | The model a request needs isn't ready yet, such as one still downloading |

## 8. Versions

The version is CNP's, `major.minor`, and audio-chat-box speaks 1.0. Programs with different
major versions don't talk. A new minor version only adds optional fields, methods,
notifications and capabilities, so an older client works with a newer node: unknown fields
are ignored, and a new capability is used only when offered. `hotwords` and `leadInMs` were
added this way, to 1.0 while it was a draft.

CNP's conformance sessions for the voice domain (`conformance/voice/` in its repository) run
against audio-chat-box in its tests (`tests/conformance.rs`).
