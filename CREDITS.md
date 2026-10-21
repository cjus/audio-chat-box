# Credits

The models `solrac-voice` downloads, and the code it links. (They moved here from Solrac's
`assets/CREDITS.md` with the voice, cjus/chessgui#98, step 6.)

## Downloaded when needed

It downloads its models the first time it needs them, from the PNX Studios CDN
(`https://cdn.pnxstudios.com/solrac/models/`), into the folder `--models` names, and checks
each against the size and SHA-256 in `src/models.rs`. The archives are sherpa-onnx's,
unchanged, and a `NOTICE.txt` beside them on the CDN gives the same licences as below.

| Archive | What | Source | Licence |
| --- | --- | --- | --- |
| `kokoro-multi-lang-v1_0.tar.bz2`, `kokoro-int8-multi-lang-v1_0.tar.bz2` | Kokoro-82M v1.0, its weights and its 54 voices | [hexgrad/Kokoro-82M](https://huggingface.co/hexgrad/Kokoro-82M), converted and packaged by [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx/releases/tag/tts-models) | Apache-2.0 |
| the same, `espeak-ng-data/` | eSpeak NG's pronunciation data | [eSpeak NG](https://github.com/espeak-ng/espeak-ng) (its source) | GPL-3.0-or-later |
| the same, `dict/` | CppJieba's dictionaries, for Chinese, which Solrac doesn't read | [CppJieba](https://github.com/yanyiwu/cppjieba) | MIT |
| `sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8.tar.bz2` | Parakeet TDT 0.6B v2: its weights converted to ONNX and quantised to 8-bit integers by sherpa-onnx, which also packaged it. `solrac-voice` writes a `bpe.vocab` beside it, made from its `tokens.txt` | [nvidia/parakeet-tdt-0.6b-v2](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v2) by NVIDIA, packaged by [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx/releases/tag/asr-models) | [CC-BY-4.0](https://creativecommons.org/licenses/by/4.0/) |

## Linked into the program

| What | Source | Licence |
| --- | --- | --- |
| sherpa-onnx's prebuilt static library: sherpa-onnx, ONNX Runtime and eSpeak NG | [k2-fsa/sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) release 1.13.8, checked by its SHA-256 in CI | Apache-2.0; ONNX Runtime MIT; eSpeak NG GPL-3.0-or-later, which makes the program as a whole GPL-3.0-or-later (README, "Licence") |

The Rust crates it depends on are permissive, and `cargo deny check` keeps them so.
