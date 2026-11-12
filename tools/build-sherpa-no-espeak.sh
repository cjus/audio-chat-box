#!/usr/bin/env bash
# Builds sherpa-onnx v1.13.8 from source with tools/no-espeak/sherpa-onnx-1.13.8.patch, which
# leaves eSpeak NG (GPL) out, into <folder> (default target/sherpa-no-espeak). Then:
#
#   SHERPA_ONNX_LIB_DIR=<folder> cargo build --release --features no-espeak
#
# tools/no-espeak/README.md says what the patch does and why this program is then
# MIT OR Apache-2.0. Needs git, cmake, a C++17 compiler and ar. The first build downloads ONNX
# Runtime and the other libraries sherpa-onnx's CMake fetches, and takes a few minutes.
set -euo pipefail

TAG=v1.13.8
COMMIT=11afbd009a7f8c08f4bcf2fc1b265d0df4670fbf

if [[ $# -gt 1 || ${1:-} == -* ]]; then
    echo "usage: tools/build-sherpa-no-espeak.sh [<folder>]" >&2
    exit 1
fi
here=$(cd "$(dirname "$0")" && pwd)
out=$(mkdir -p "${1:-target/sherpa-no-espeak}" && cd "${1:-target/sherpa-no-espeak}" && pwd)
src="$out/src"
build="$out/build"

if [[ ! -d "$src/.git" ]]; then
    git clone --quiet --depth 1 --branch "$TAG" https://github.com/k2-fsa/sherpa-onnx "$src"
fi
if [[ $(git -C "$src" rev-parse HEAD) != "$COMMIT" ]]; then
    echo "sherpa-onnx $TAG is not commit $COMMIT: refusing to build it" >&2
    exit 1
fi
git -C "$src" checkout --quiet -- .
git -C "$src" clean --quiet -fd
git -C "$src" apply "$here/no-espeak/sherpa-onnx-1.13.8.patch"

case "$(uname -s)" in
    MINGW* | MSYS* | CYGWIN*) windows=1 ;;
    *) windows=0 ;;
esac
generator=()
if [[ $windows == 1 ]]; then
    generator=(-G Ninja)
fi

cmake -S "$src" -B "$build" ${generator[@]+"${generator[@]}"} -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=OFF \
    -DSHERPA_ONNX_ENABLE_C_API=ON -DSHERPA_ONNX_ENABLE_TTS=ON \
    -DSHERPA_ONNX_ENABLE_BINARY=OFF -DSHERPA_ONNX_ENABLE_PORTAUDIO=OFF \
    -DSHERPA_ONNX_ENABLE_WEBSOCKET=OFF -DSHERPA_ONNX_ENABLE_CHECK=OFF \
    -DSHERPA_ONNX_BUILD_C_API_EXAMPLES=OFF -DSHERPA_ONNX_ENABLE_SPEAKER_DIARIZATION=ON \
    -DSHERPA_ONNX_LINK_LIBSTDCPP_STATICALLY=ON -DSHERPA_ONNX_USE_STATIC_CRT=ON
cmake --build "$build" --config Release -j "$(getconf _NPROCESSORS_ONLN)"

lib="$out/lib"
rm -rf "$lib"
mkdir -p "$lib"
if [[ $windows == 1 ]]; then
    # Single-config Ninja: the libraries are where CMake put them, named <name>.lib.
    find "$build" -name '*.lib' -not -path '*/CMakeFiles/*' -exec cp {} "$lib/" \;
    cp "$build/_deps/onnxruntime-src/lib/onnxruntime.lib" "$lib/"
    # sherpa-onnx-sys links these two by name; they hold nothing (no eSpeak NG here).
    : > "$out/empty.c"
    (cd "$out" && cl -nologo -MT -c empty.c -Fo:empty.obj)
    for name in espeak-ng ucd; do
        rm -f "$lib/$name.lib"
        lib -nologo "-OUT:$lib/$name.lib" "$out/empty.obj"
    done
else
    cp "$build"/lib/*.a "$build"/_deps/onnxruntime-src/lib/libonnxruntime.a "$lib/"
    # sherpa-onnx-sys links these two by name; they hold nothing (no eSpeak NG here).
    # (An archive needs a member on macOS, so each holds one empty object file.)
    : > "$out/empty.c"
    cc -c "$out/empty.c" -o "$out/empty.o"
    for name in espeak-ng ucd; do
        rm -f "$lib/lib$name.a"
        ar rcs "$lib/lib$name.a" "$out/empty.o"
    done
fi
echo "sherpa-onnx $TAG, patched, without eSpeak NG (tools/no-espeak/README.md)" > "$lib/NO-ESPEAK"
echo "built: SHERPA_ONNX_LIB_DIR=$lib"
