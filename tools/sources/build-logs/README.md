# The logs of the builds that made sherpa-onnx's static libraries

GitHub keeps a run's log for 90 days, so the logs of the runs that built the prebuilt
libraries `audio-chat-box` links (sherpa-onnx 1.13.8, and the ONNX Runtime 1.28.2 in it) are
kept here, compressed (`gunzip -k`), as `gh run view` gave them on 2026-10-29. They are the
evidence for what `tools/sources/sources.txt` and `tools/sources/README.md` say each build
fetched.

## sherpa-onnx (`k2-fsa/sherpa-onnx`, tag `v1.13.8`)

| Log | Workflow, job | Made |
| --- | --- | --- |
| `sherpa-onnx-run-34486043450-job-102900467207-linux-x64-static.log.gz` | `linux.yaml`, "Release shared-OFF tts-ON" | `sherpa-onnx-v1.13.8-linux-x64-static-lib.tar.bz2` |
| `sherpa-onnx-run-34486043530-job-102900466842-linux-aarch64-static.log.gz` | `aarch64-linux-gnu-static.yaml`, "aarch64 static lib test (ubuntu-22.04-arm)" | `sherpa-onnx-v1.13.8-linux-aarch64-static-lib.tar.bz2` |
| `sherpa-onnx-run-34486043393-job-102900467913-macos-universal2-static.log.gz` | `macos.yaml`, "Release static tts-ON macos-latest arm64;x86_64" | `sherpa-onnx-v1.13.8-osx-universal2-static-lib.tar.bz2`, and from it (`lipo -thin`) `-osx-arm64-` and `-osx-x64-static-lib.tar.bz2` |

Each finished within seconds of its files' upload to the release. Their "Downloading ... from"
lines are the archives in `sources.txt`, and the ONNX Runtime each took:
`onnxruntime-linux-x64-static_lib-1.28.2-glibc2_17.zip`,
`onnxruntime-linux-aarch64-static_lib-1.28.2-glibc2_17.zip` and
`onnxruntime-osx-universal2-static_lib-1.28.2.zip`.

## ONNX Runtime (`csukuangfj/onnxruntime-build`, released in `csukuangfj/onnxruntime-libs`)

| Log | Workflow | Made |
| --- | --- | --- |
| `onnxruntime-build-run-33741118168-linux-x64-glibc2_17.log.gz` | `linux-217.yaml` | `onnxruntime-linux-x64-static_lib-1.28.2-glibc2_17.zip` (linked on Linux x86-64) |
| `onnxruntime-build-run-33741118071-linux-aarch64-glibc2_17.log.gz` | `aarch64-217.yaml` | `onnxruntime-linux-aarch64-static_lib-1.28.2-glibc2_17.zip` (linked on Linux arm64) |
| `onnxruntime-build-run-33741118112-macos-universal2.log.gz` | `macos-universal2.yaml` | `onnxruntime-osx-universal2-static_lib-1.28.2.zip` (linked on macOS) |
| `onnxruntime-build-run-33741118063-macos-arm64.log.gz` | `macos-arm64.yaml` | `onnxruntime-osx-arm64-static_lib-1.28.2.zip` (not linked) |
| `onnxruntime-build-run-33741118056-macos-x64.log.gz` | `macos-x64.yaml` | `onnxruntime-osx-x86_64-static_lib-1.28.2.zip` (not linked) |

All five ran onnxruntime-build at `6314c7b`, built Microsoft's `v1.28.2` (`33ca962`) with the
same submodules, and fetched the same twelve dependencies ("-- Fetch ... from"), but for the
universal2 build, which didn't fetch cpuinfo. Each finished within a minute after its file was
uploaded to onnxruntime-libs. sherpa-onnx's macOS libraries link the universal2 build, not the
single-architecture ones, which are kept for completeness.
