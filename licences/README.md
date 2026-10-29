# The native libraries' licences

sherpa-onnx's build script links a prebuilt static library into `audio-chat-box` (the
`sherpa-onnx` pin in `Cargo.toml`, checked by its SHA-256 in CI). That archive holds the
compiled libraries below and none of their licence files, so their licences are kept here,
each taken from the project's source at the version sherpa-onnx 1.13.8's CMake files build
(`cmake/*.cmake` at its tag `v1.13.8`, and kaldi-decoder's at `v0.3.0`). `tools/notices.sh`
puts them all in `THIRD-PARTY-NOTICES.txt`, after the crates'.

eSpeak NG makes the program as a whole GPL-3.0-or-later (`LICENSE-GPL-3.0`, the README's
"Licence"). Every other library here has a licence the GPL allows alongside it.

| Folder | Library (static library in the archive) | Version | Source | Licence |
| --- | --- | --- | --- | --- |
| `sherpa-onnx` | sherpa-onnx (`sherpa-onnx-c-api`, `sherpa-onnx-core`) | 1.13.8 | <https://github.com/k2-fsa/sherpa-onnx> | Apache-2.0 |
| `onnxruntime` | ONNX Runtime (`onnxruntime`), with the libraries it builds in, which its `ThirdPartyNotices.txt` lists | 1.28.2, built by <https://github.com/csukuangfj/onnxruntime-build> (commit `6314c7b`) from Microsoft's tag `v1.28.2`, released in <https://github.com/csukuangfj/onnxruntime-libs> | <https://github.com/microsoft/onnxruntime> | MIT |
| `kaldi-decoder` | kaldi-decoder (`kaldi-decoder-core`) | 0.3.0 | <https://github.com/k2-fsa/kaldi-decoder> | Apache-2.0 |
| `kaldifst` | kaldifst (`sherpa-onnx-kaldifst-core`), through kaldi-decoder | 1.8.0 | <https://github.com/k2-fsa/kaldifst> | Apache-2.0 |
| `openfst` | OpenFst (`sherpa-onnx-fst`, `sherpa-onnx-fstfar`) | 1.8.5, sherpa-onnx's fork, tag `v1.8.5-2026-07-09` | <https://github.com/csukuangfj/openfst> | Apache-2.0 |
| `kaldi-native-fbank` | kaldi-native-fbank (`kaldi-native-fbank-core`) | 1.22.3 | <https://github.com/csukuangfj/kaldi-native-fbank> | Apache-2.0 |
| `kissfft` | KISS FFT (`kissfft-float`), through kaldi-native-fbank | commit `febd4ca` | <https://github.com/mborgerding/kissfft> | BSD-3-Clause |
| `piper-phonemize` | piper-phonemize (`piper_phonemize`), with uni-algo, which it builds in | commit `f3ff95a` of sherpa-onnx's fork | <https://github.com/csukuangfj/piper-phonemize> | MIT; uni-algo MIT or public domain |
| `espeak-ng` | eSpeak NG (`espeak-ng`), and its ucd-tools (`ucd`), whose `COPYING` is the same GPL text | commit `ed530aa` of sherpa-onnx's fork | <https://github.com/csukuangfj/espeak-ng> | GPL-3.0-or-later; parts Apache-2.0, BSD-2-Clause and the Unicode data licence |
| `simple-sentencepiece` | simple-sentencepiece (`ssentencepiece_core`) | 0.7 | <https://github.com/pkufool/simple-sentencepiece> | Apache-2.0 |
| `nlohmann-json` | JSON for Modern C++, header-only, in sherpa-onnx's core | 3.12.0 | <https://github.com/nlohmann/json> | MIT |
| `hclust-cpp` | hclust-cpp (fastcluster), compiled into sherpa-onnx's core for speaker diarization | tag `2026-02-25` of sherpa-onnx's fork | <https://github.com/csukuangfj/hclust-cpp> | BSD-2-Clause |
| `eigen` | Eigen, header-only, through kaldi-decoder: all its licence files, since some of its headers are BSD-licensed and which ones are compiled in isn't known. kaldi-decoder's `cmake/` names 3.4.0, but sherpa-onnx's `cmake/` comes first in CMake's module path, so its `eigen.cmake` is the one used | 5.0.1 | <https://gitlab.com/libeigen/eigen> | MPL-2.0, parts BSD, Apache-2.0 and MINPACK |

The archive also holds `sherpa-onnx-portaudio_static` (PortAudio) and `sherpa-onnx-cxx-api`,
which `audio-chat-box` doesn't link: `sherpa-onnx-sys` names the libraries it links, and
neither is among them.

**Their source** is the `sources-v1` release of this repository, which
[`tools/sources.sh`](../tools/sources.sh) gathers from
[`tools/sources/sources.txt`](../tools/sources/sources.txt): every archive the builds of the
Linux x86-64 and macOS (arm64, x86-64) libraries fetched, each checked by upstream's hash,
with how each library was built ([`tools/sources/README.md`](../tools/sources/README.md)) and
the logs of those builds ([`tools/sources/build-logs/`](../tools/sources/build-logs/README.md)). Every copy of the program
must be able to lead to it, as the program is GPL-3.0-or-later.

**When the sherpa-onnx pin moves**, look again at its `cmake/*.cmake` at the new tag (and
kaldi-decoder's at the version it names), update the versions above, and take each licence
file again from its source at the new version. Then update `tools/sources/sources.txt`,
including which build made the prebuilt ONNX Runtime, and `tools/sources/README.md`, which
names its release (it is the image's `SOURCES.md`), run `tools/sources.sh`, and make a new
`sources-v<n>` release.
