# The logs of the builds that made the prebuilt ONNX Runtime

sherpa-onnx 1.13.8 links an ONNX Runtime 1.28.2 that `csukuangfj/onnxruntime-build` built
and `csukuangfj/onnxruntime-libs` released. GitHub keeps a run's log for 90 days, so the logs of
the runs that built it are kept here, compressed (`gunzip -k`), as `gh run view <run> -R
csukuangfj/onnxruntime-build --log` gave them on 2026-10-29. They are the evidence for what
`tools/sources/sources.txt` says each build fetched.

| Log | Workflow | Made |
| --- | --- | --- |
| `onnxruntime-build-run-33741118168-linux-x64-glibc2_17.log.gz` | `linux-217.yaml` | `onnxruntime-linux-x64-static_lib-1.28.2-glibc2_17.zip` |
| `onnxruntime-build-run-33741118063-macos-arm64.log.gz` | `macos-arm64.yaml` | `onnxruntime-osx-arm64-static_lib-1.28.2.zip` |
| `onnxruntime-build-run-33741118056-macos-x64.log.gz` | `macos-x64.yaml` | `onnxruntime-osx-x86_64-static_lib-1.28.2.zip` |
| `onnxruntime-build-run-33741118112-macos-universal2.log.gz` | `macos-universal2.yaml` | `onnxruntime-osx-universal2-static_lib-1.28.2.zip` |

All four ran onnxruntime-build at `6314c7b`, built Microsoft's `v1.28.2` (`33ca962`) with the
same submodules, and fetched the same twelve dependencies ("-- Fetch ... from"), but for the
universal2 build, which didn't fetch cpuinfo. Each finished within a minute after its file was
uploaded to onnxruntime-libs.
