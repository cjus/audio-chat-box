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
  Numbers ("4", "1972", "1990s", "2nd") are said as English words (a year as a year), since
  eSpeak read digits and Solrac says each move's rank as one; a compound the lexicon lacks
  ("middlegame") is two words it has; a square or code written as one token ("f3", "e6",
  "mp3") is its letters by name, then the number.
  The model's config no longer needs `espeak-ng-data` (`offline-tts-kokoro-model-config.cc`).
- **Pronunciations per request.** A generation config's `extra["pronunciations"]` holds
  lexicon lines ("word sym sym ...") that win over the lexicon's for that request
  (`offline-tts-kokoro-impl.h`, `kokoro-multi-lang-lexicon.*`). This is what CNP's
  `voice/speak` `pronunciations` become (`src/pronounce.rs`).

The patched library is not shipped with the usual build, and the model's lexicon files (the
`lexicon-*.txt` of kokoro-multi-lang-v1_0) are used as they are. Every other library in it is
sherpa-onnx's own, at the versions in `licences/README.md`, minus `espeak-ng` and
`piper-phonemize`.

## The model archive

The usual Kokoro archive holds `espeak-ng-data/` (GPL-3.0-or-later). The `no-espeak` build
downloads another one, `kokoro-multi-lang-v1_0-no-espeak.tar.bz2` (and an int8 one), which is
sherpa-onnx's archive with that folder left out and the folder renamed to match
(`tools/pack-kokoro-no-espeak.sh`, which checks its inputs by SHA-256). `src/models.rs` names
them, with their sizes and checksums, only with the feature, and the Kokoro config no longer
asks for the data folder (`src/speak.rs`). The archives are in the `models-v1` release, and
a product with its own copy of the models (Solrac's) puts them there too.

## On an iPad

```text
tools/build-sherpa-no-espeak.sh --ios [<folder>]       # default: target/sherpa-no-espeak-ios
tools/build-sherpa-no-espeak.sh --ios-sim [<folder>]   # the Simulator on an Apple silicon Mac
SHERPA_ONNX_LIB_DIR=<folder>/lib IPHONEOS_DEPLOYMENT_TARGET=17.0 \
    cargo --config <folder>/cargo-config.toml build --release --target aarch64-apple-ios --features no-espeak
```

An iPad app can't start a program, so there this build is linked into the app as a library
(cjus/chessgui#687, #698; README, "Licence"). The same patched sherpa-onnx is built with its
own iOS toolchain file (`toolchains/ios.toolchain.cmake`, as its `build-ios.sh` does), for
iPadOS `IPHONEOS_DEPLOYMENT_TARGET` (default 17.0), against ONNX Runtime 1.28.2's static iOS
xcframework from the same builds `licences/README.md` names, checked by its SHA-256. So the
libraries and their licences are the Mac's.

Three things differ from the Mac's build, each handled by the script:

- **Static linking through an override.** sherpa-onnx-sys 1.13.8 links iOS only as a shared
  library, whatever its `static` feature says, and fails without a `.dylib`. The script
  writes `cargo-config.toml`, which overrides that build script (Cargo's override of a
  `links` build script: sherpa-onnx-sys says `links = "sherpa-onnx"`) with the static
  libraries it would have named. Pass it with `cargo --config`.
- **clang's runtime.** ONNX Runtime's Core ML code calls `___isPlatformVersionAtLeast`
  (Objective-C's `@available`), which is in clang's `libclang_rt.ios.a`, not in what rustc
  links. The config links it, unbundled (`static:-bundle`: rustc can't read a universal
  archive), and the Core ML framework.
- **The empty `espeak-ng` and `ucd` archives** are built for iOS, so the linker sees no Mac
  object in an iPad app.

## Why it can't be mistaken for the usual build

`sherpa-onnx-sys` links `espeak-ng` and `ucd` by name, so the script puts two empty archives
with those names beside the real libraries, and a marker file, `NO-ESPEAK`. `build.rs` refuses
to build with the `no-espeak` feature unless `SHERPA_ONNX_LIB_DIR` holds that marker, and the
node says `MIT OR Apache-2.0` as its licence only then (`node::LICENSE`).
