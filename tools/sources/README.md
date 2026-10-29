# The source of audio-chat-box's native libraries

`audio-chat-box` links sherpa-onnx's prebuilt static library, which includes
[eSpeak NG](https://github.com/espeak-ng/espeak-ng) (GPL-3.0-or-later), so the program as built
is GPL-3.0-or-later. Whoever has a copy of it may have its Corresponding Source: the
[audio-chat-box repository](https://github.com/cjus/audio-chat-box) at the commit it was built
from, and the source in this release of every library in that static library the program
links, as the builds that made the library fetched them. Each archive keeps its own licence.

**This release is <https://github.com/cjus/audio-chat-box/releases/tag/sources-v1>**: every
archive named below can be downloaded from it, with `SHA256SUMS`.

It covers every copy of `audio-chat-box` built with sherpa-onnx **1.13.8** (`Cargo.toml`) for
**Linux** (x86-64 and arm64) and **macOS** (Apple silicon and Intel), whose libraries are:

| Platform | sherpa-onnx's static library | SHA-256 |
| --- | --- | --- |
| Linux x86-64 | `sherpa-onnx-v1.13.8-linux-x64-static-lib.tar.bz2` | `e1fdc5b67530e15741ef897fa5ffff297056f3bf0c6d829a27af9225a4c4b5a6` |
| Linux arm64 | `sherpa-onnx-v1.13.8-linux-aarch64-static-lib.tar.bz2` | `77983e3cf29aa60f2e531d249dbd01d15596530550c8db2e9e02fc6a655da6bb` |
| macOS arm64 | `sherpa-onnx-v1.13.8-osx-arm64-static-lib.tar.bz2` | `9091bf160dc7fdacedbc906b212badf53c2993f4e5277a0e03998e96c31d60da` |
| macOS x86-64 | `sherpa-onnx-v1.13.8-osx-x64-static-lib.tar.bz2` | `a3f88da3e54c850a12d61431e73f8affcd1f13738b75b768847dd79541835b4b` |

That includes the Docker image `pnxstudios/audio-chat-box-demo` from `0.1.0` (commit `4190c28`)
on: its `org.opencontainers.image.revision` label names the commit of each.

## What is in it

| Library in the static library | Source here |
| --- | --- |
| `libsherpa-onnx-c-api.a`, `libsherpa-onnx-core.a` | `sherpa-onnx-1.13.8.tar.gz`, with `nlohmann-json-3.12.0.tar.gz` and `hclust-cpp-2026-02-25.tar.gz` compiled in |
| `libkaldi-decoder-core.a` | `kaldi-decoder-0.3.0.tar.gz`, with `eigen-5.0.1.tar.gz` (header-only) |
| `libsherpa-onnx-kaldifst-core.a` | `kaldifst-1.8.0.tar.gz` |
| `libsherpa-onnx-fst.a`, `libsherpa-onnx-fstfar.a` | `openfst-1.8.5-2026-07-09.tar.gz` |
| `libkaldi-native-fbank-core.a` | `kaldi-native-fbank-1.22.3.tar.gz` |
| `libkissfft-float.a` | `kissfft-febd4caeed32.zip` |
| `libpiper_phonemize.a` | `piper-phonemize-f3ff95afc036.zip` (with uni-algo in it) |
| `libespeak-ng.a`, `libucd.a` | `espeak-ng-ed530aa11304.zip` |
| `libssentencepiece_core.a` | `simple-sentencepiece-0.7.tar.gz` |
| `libonnxruntime.a` | `onnxruntime-1.28.2.tar.gz` and its three `onnxruntime-1.28.2-submodule-*` archives, the `onnxruntime-dep-*` archives its build fetched (all twelve on Linux; on macOS all but cpuinfo), and `onnxruntime-build-6314c7b.tar.gz`, which built it |

The Linux and macOS libraries hold the same fifteen static libraries. They also hold
`libsherpa-onnx-portaudio_static.a` and `libsherpa-onnx-cxx-api.a`, which `audio-chat-box`
doesn't link (`sherpa-onnx-sys` names the libraries it links, and neither is among them), so
PortAudio's source isn't here. `sherpa-onnx-1.13.8.tar.gz` has the C++ API's anyway.

## How each was built

Every one is built from sherpa-onnx's CMake files (`cmake/*.cmake`) at `v1.13.8`, which fetch
the archives above, each checked by the SHA-256 they give it. kaldi-decoder's own `cmake/`
would fetch Eigen 3.4.0 and kaldifst's OpenFst 2026-04-10, but sherpa-onnx's `cmake/` comes
first in CMake's module path, so its `eigen.cmake` (5.0.1) and `openfst.cmake` (2026-07-09)
are the ones used (each build's log shows it).

- **sherpa-onnx's static library for Linux x86-64**: its `.github/workflows/linux.yaml`
  (GitHub Actions run 34486043450, job "Release shared-OFF tts-ON"), in the
  `quay.io/pypa/manylinux2014_x86_64` image, with shared libraries off.
- **sherpa-onnx's static library for Linux arm64**: its
  `.github/workflows/aarch64-linux-gnu-static.yaml` (run 34486043530, job "aarch64 static lib
  test (ubuntu-22.04-arm)"), built on an arm64 Ubuntu 22.04 runner.
- **sherpa-onnx's static libraries for macOS**: its `.github/workflows/macos.yaml` (run
  34486043393, job "Release static tts-ON macos-latest arm64;x86_64"), which builds one
  universal library for both architectures, then cuts the arm64 and x86-64 libraries out of it
  with `lipo -thin`. Being universal, it links the universal2 ONNX Runtime
  (`cmake/onnxruntime-osx-universal-static.cmake`).
- **ONNX Runtime 1.28.2**, which sherpa-onnx links prebuilt from `csukuangfj/onnxruntime-libs`:
  `csukuangfj/onnxruntime-build` at commit `6314c7b` built each from Microsoft's tag `v1.28.2`
  (commit `33ca962`) with its submodules, after applying that repository's patch to ONNX
  Runtime's CMake files; protoc 21.12 ran as a build tool. Each build fetched the
  `onnxruntime-dep-*` archives ("-- Fetch ... from" in its log); the rest of ONNX Runtime's
  `cmake/deps.txt` (GPU, mobile, web and test dependencies) wasn't fetched.

  | Platform | ONNX Runtime (`onnxruntime-libs` `v1.28.2`) | Built by | Fetched |
  | --- | --- | --- | --- |
  | Linux x86-64 | `onnxruntime-linux-x64-static_lib-1.28.2-glibc2_17.zip` | `linux-217.yaml`, run 33741118168, in `quay.io/pypa/manylinux2014_x86_64` | all twelve |
  | Linux arm64 | `onnxruntime-linux-aarch64-static_lib-1.28.2-glibc2_17.zip` | `aarch64-217.yaml`, run 33741118071 | all twelve |
  | macOS | `onnxruntime-osx-universal2-static_lib-1.28.2.zip` | `macos-universal2.yaml`, run 33741118112 | all but cpuinfo |

  Each library's `libonnxruntime.a` is that build's, byte for byte (on macOS, its arm64 slice
  is the arm64 library's).

The logs of all these runs are in the audio-chat-box repository, in
[`tools/sources/build-logs/`](https://github.com/cjus/audio-chat-box/tree/main/tools/sources/build-logs),
as GitHub keeps a run's log for only 90 days.

## eSpeak NG's pronunciation data

Kokoro's model archive carries eSpeak NG's compiled pronunciation data (`espeak-ng-data/`,
GPL-3.0-or-later), which sherpa-onnx copied in from the `espeak-ng-data.tar.bz2` of its
`tts-models` release (2023-11-29). Neither records what compiled it, so this was found by
compiling candidates and comparing them with the data in
`kokoro-multi-lang-v1_0.tar.bz2` (the one in this repository's `models-v1` release):

- **Its source is `espeak-ng-8593723f10cf.zip`**: sherpa-onnx's fork of eSpeak NG at
  `8593723` (2023-11-27, the head of the fork and of rhasspy's when the data was published),
  built with CMake. Of the 355 files, 343 are byte for byte the same as that build's.
- The other 12 (the `bn`, `ja`, `shn`, `my`, `kok`, `sd`, `chr`, `ko`, `vi`, `as`, `da`
  and `th` dictionaries) are the same size and hold exactly the same bytes in another order,
  near the end of each, where the compiler sorts its rules. Builds elsewhere differ the same
  way (rhasspy's piper-phonemize 2023.11.14-4 data does too), while two builds in one
  environment are identical: the C library's sorting orders equal entries its own way.
- Built with autotools, the same source gives a different `fa_dict` (293,235 bytes against
  292,423), so the CMake build is the one that made it.
- `espeak-ng-ed530aa11304.zip`, which the program links, is 9 commits on from `8593723`,
  none of them to the data's sources (`dictsource/`, `phsource/`): built the same way in the
  same environment, the two give byte-identical data.

To compile it: `cmake -S . -B build -DCMAKE_BUILD_TYPE=Release && cmake --build build` in the
unpacked archive puts it in `build/espeak-ng-data`.

## Checking it

`SHA256SUMS` has the SHA-256 of every file here. Each archive is also the one upstream pins,
by the hash in [`tools/sources/sources.txt`](https://github.com/cjus/audio-chat-box/blob/main/tools/sources/sources.txt):
sherpa-onnx's, kaldi-decoder's and kaldi-native-fbank's CMake files (SHA-256) and ONNX
Runtime's `cmake/deps.txt` (SHA-1); `audio-chat-box` pinned the GitHub archives of
sherpa-onnx, ONNX Runtime and its submodules, and onnxruntime-build itself.
`tools/sources.sh` downloads them all again and checks them.

The GNU General Public License, version 3, is `LICENSE-GPL-3.0` in the audio-chat-box
repository.
