# The build without eSpeak NG

The usual `audio-chat-box` links sherpa-onnx's prebuilt library, which includes eSpeak NG
(GPL-3.0-or-later), so the program as a whole is GPL-3.0-or-later. This folder builds the
other flavour: sherpa-onnx v1.13.8 with a patch, from source, without eSpeak NG or
piper-phonemize. That program is `MIT OR Apache-2.0` as a whole (README, "Licence").

```text
tools/build-sherpa-no-espeak.sh [<folder>]     # default: target/sherpa-no-espeak
SHERPA_ONNX_LIB_DIR=<folder> cargo build --release --features no-espeak
```

## What the patch changes

`sherpa-onnx-1.13.8.patch` is a change to k2-fsa's sherpa-onnx (Apache-2.0), made by
Carlos Justiniano and Claude in November 2026, as Apache-2.0 section 4(b) asks us to say:

- **No eSpeak NG.** CMake builds a stand-in `piper_phonemize` library (`stub-piper/`, written
  from scratch, no eSpeak code in it) instead of eSpeak NG and piper-phonemize. Kokoro
  (`kokoro-multi-lang-lexicon.cc`) no longer sounds out words with eSpeak: a word the
  lexicon lacks gets its inflection rules (plural -s, -ed, -ing, -ly, -er, rewritten from
  the idea in misaki, Apache-2.0), or is skipped. "the" and "a" take their weak forms.
  The model's config no longer needs `espeak-ng-data` (`offline-tts-kokoro-model-config.cc`).
- **Pronunciations per request.** A generation config's `extra["pronunciations"]` holds
  lexicon lines ("word sym sym ...") that win over the lexicon's for that request
  (`offline-tts-kokoro-impl.h`, `kokoro-multi-lang-lexicon.*`). This is what CNP's
  `voice/speak` `pronunciations` become (`src/pronounce.rs`).

The patched library is not shipped with the usual build, and the model's lexicon files (the
`lexicon-*.txt` of kokoro-multi-lang-v1_0) are used as they are. Every other library in it is
sherpa-onnx's own, at the versions in `licences/README.md`, minus `espeak-ng` and
`piper-phonemize`.

## Why it can't be mistaken for the usual build

`sherpa-onnx-sys` links `espeak-ng` and `ucd` by name, so the script puts two empty archives
with those names beside the real libraries, and a marker file, `NO-ESPEAK`. `build.rs` refuses
to build with the `no-espeak` feature unless `SHERPA_ONNX_LIB_DIR` holds that marker, and the
node says `MIT OR Apache-2.0` as its licence only then (`node::LICENSE`).
