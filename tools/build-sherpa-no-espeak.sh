#!/usr/bin/env bash
# Builds sherpa-onnx v1.13.8 from source with tools/no-espeak/sherpa-onnx-1.13.8.patch, which
# leaves eSpeak NG (GPL) out, into <folder> (default target/sherpa-no-espeak). Then:
#
#   SHERPA_ONNX_LIB_DIR=<folder>/lib cargo build --release --features no-espeak
#
# With --ios or --ios-sim it builds for an iPad (aarch64-apple-ios) or the Simulator on an
# Apple silicon Mac (aarch64-apple-ios-sim) instead, where a program can't be started and the
# library is linked into the app (cjus/chessgui#687, #698), into target/sherpa-no-espeak-ios or
# -ios-sim by default. IPHONEOS_DEPLOYMENT_TARGET (default 17.0) is the oldest iPadOS it runs
# on, and should be the app's. Then, from the app's build:
#
#   SHERPA_ONNX_LIB_DIR=<folder>/lib IPHONEOS_DEPLOYMENT_TARGET=17.0 \
#       cargo build --release --target aarch64-apple-ios --features no-espeak
#
# tools/no-espeak/README.md says what the patch does and why this program is then
# MIT OR Apache-2.0. Needs git, cmake, a C++17 compiler and ar. The first build downloads ONNX
# Runtime and the other libraries sherpa-onnx's CMake fetches, and takes a few minutes.
set -euo pipefail

TAG=v1.13.8
COMMIT=11afbd009a7f8c08f4bcf2fc1b265d0df4670fbf

# ONNX Runtime for iOS: sherpa-onnx's CMake downloads it for the other platforms, but its
# build-ios.sh takes this static xcframework, the same version from the same builds that
# licences/README.md names. Checked by its SHA-256 before it is unpacked.
ORT_VERSION=1.28.2
ORT_IOS_URL=https://github.com/csukuangfj/onnxruntime-libs/releases/download/v$ORT_VERSION/onnxruntime-ios-static-xcframework-$ORT_VERSION.xcframework.zip
ORT_IOS_SHA256=2c2299acbb461d26d4bac4bc85985d40e7c7177ed6072703ae0846d88b0b4599

usage() {
    echo "usage: tools/build-sherpa-no-espeak.sh [--ios | --ios-sim] [<folder>]" >&2
    exit 1
}
platform=host
if [[ ${1:-} == --ios || ${1:-} == --ios-sim ]]; then
    platform=${1#--}
    shift
fi
if [[ $# -gt 1 || ${1:-} == -* ]]; then
    usage
fi
case $platform in
    host) default_out=target/sherpa-no-espeak ;;
    *) default_out=target/sherpa-no-espeak-$platform ;;
esac
here=$(cd "$(dirname "$0")" && pwd)
out=$(mkdir -p "${1:-$default_out}" && cd "${1:-$default_out}" && pwd)
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
# Windows: CMake's newest Visual Studio generator, 64-bit (sherpa-onnx's ONNX Runtime
# download checks the platform name, which Ninja doesn't set), so the libraries land in a
# per-configuration folder, and are found by name below.
generator=()
if [[ $windows == 1 ]]; then
    generator=(-A x64)
fi

# iOS: sherpa-onnx's own toolchain file (leetal's ios-cmake, BSD-3-Clause, in its
# toolchains/), as its build-ios.sh uses it, and the ONNX Runtime above in place of the
# download. The xcframework's library is a static archive inside a framework, wrapped as a
# universal file even with one architecture: `lipo -thin` makes it the plain archive
# sherpa-onnx-sys links as `onnxruntime`.
ios=()
if [[ $platform != host ]]; then
    [[ $(uname -s) == Darwin ]] || { echo "--$platform needs a Mac with Xcode" >&2; exit 1; }
    deployment_target=${IPHONEOS_DEPLOYMENT_TARGET:-17.0}
    if [[ $platform == ios ]]; then
        ios_platform=OS64 slice=ios-arm64 sdk=iphoneos min_flag=-mios-version-min
    else
        ios_platform=SIMULATORARM64 slice=ios-arm64_x86_64-simulator sdk=iphonesimulator
        min_flag=-mios-simulator-version-min
    fi
    ort="$out/onnxruntime-$ORT_VERSION"
    if [[ ! -d "$ort/onnxruntime.xcframework" ]]; then
        rm -rf "$ort"
        mkdir -p "$ort"
        curl -fsSL "$ORT_IOS_URL" -o "$ort/onnxruntime.zip"
        sum=$(shasum -a 256 "$ort/onnxruntime.zip" | cut -d' ' -f1)
        if [[ $sum != "$ORT_IOS_SHA256" ]]; then
            rm -rf "$ort"
            echo "ONNX Runtime for iOS: SHA-256 $sum, not $ORT_IOS_SHA256: refusing it" >&2
            exit 1
        fi
        unzip -q "$ort/onnxruntime.zip" -d "$ort"
        rm "$ort/onnxruntime.zip"
    fi
    framework="$ort/onnxruntime.xcframework/$slice/onnxruntime.framework"
    ort_lib="$out/onnxruntime-lib"
    rm -rf "$ort_lib"
    mkdir -p "$ort_lib"
    lipo -thin arm64 "$framework/onnxruntime" -output "$ort_lib/libonnxruntime.a"
    export SHERPA_ONNXRUNTIME_LIB_DIR="$ort_lib"
    export SHERPA_ONNXRUNTIME_INCLUDE_DIR="$framework/Headers"
    ios=(-DCMAKE_TOOLCHAIN_FILE="$src/toolchains/ios.toolchain.cmake" -DPLATFORM=$ios_platform
        -DDEPLOYMENT_TARGET="$deployment_target" -DENABLE_BITCODE=0 -DENABLE_ARC=1
        -DENABLE_VISIBILITY=0 -DSHERPA_ONNX_ENABLE_JNI=OFF -DSHERPA_ONNX_ENABLE_PYTHON=OFF
        -DSHERPA_ONNX_ENABLE_TESTS=OFF)
fi

cmake -S "$src" -B "$build" ${generator[@]+"${generator[@]}"} ${ios[@]+"${ios[@]}"} -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=OFF \
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
    # The libraries (ONNX Runtime's among them) are named <name>.lib.
    find "$build" -name '*.lib' -not -path '*/CMakeFiles/*' -exec cp {} "$lib/" \;
    # sherpa-onnx-sys links these two by name; they hold nothing (no eSpeak NG here).
    : > "$out/empty.c"
    (cd "$out" && cl -nologo -MT -c empty.c -Fo:empty.obj)
    for name in espeak-ng ucd; do
        rm -f "$lib/$name.lib"
        lib -nologo "-OUT:$lib/$name.lib" "$out/empty.obj"
    done
else
    cp "$build"/lib/*.a "$lib/"
    if [[ $platform == host ]]; then
        cp "$build"/_deps/onnxruntime-src/lib/libonnxruntime.a "$lib/"
    else
        cp "$SHERPA_ONNXRUNTIME_LIB_DIR/libonnxruntime.a" "$lib/"
    fi
    # sherpa-onnx-sys links these two by name; they hold nothing (no eSpeak NG here).
    # (An archive needs a member on macOS, so each holds one empty object file, built for
    # the same platform as the rest, so the linker never warns of a Mac object in an iPad app.)
    : > "$out/empty.c"
    if [[ $platform == host ]]; then
        cc -c "$out/empty.c" -o "$out/empty.o"
    else
        xcrun --sdk $sdk clang -arch arm64 "$min_flag=$deployment_target" -c "$out/empty.c" \
            -o "$out/empty.o"
    fi
    for name in espeak-ng ucd; do
        rm -f "$lib/lib$name.a"
        ar rcs "$lib/lib$name.a" "$out/empty.o"
    done
fi
echo "sherpa-onnx $TAG, patched, without eSpeak NG (tools/no-espeak/README.md), for $platform" > "$lib/NO-ESPEAK"
echo "built: SHERPA_ONNX_LIB_DIR=$lib"

# iOS: sherpa-onnx-sys's build script links iOS only as a shared library (1.13.8 ignores its
# `static` feature there) and finds no .dylib here. An iPad app links it statically instead,
# through Cargo's override of a `links` build script (sherpa-onnx-sys says
# `links = "sherpa-onnx"`): this file names the libraries in its place, as its build script
# would for a static build. Pass it to the app's build with `cargo --config <file>`.
# ONNX Runtime for iOS also needs clang's runtime (`___isPlatformVersionAtLeast`, for its
# `@available` checks), which rustc doesn't link, and the Core ML framework its Core ML
# execution provider calls.
if [[ $platform != host ]]; then
    triple=aarch64-apple-$platform
    config="$out/cargo-config.toml"
    clang_rt="$(xcrun clang --print-resource-dir)/lib/darwin"
    if [[ $platform == ios ]]; then rt=clang_rt.ios; else rt=clang_rt.iossim; fi
    {
        echo "# Made by audio-chat-box's tools/build-sherpa-no-espeak.sh --$platform: links the"
        echo "# patched sherpa-onnx (no eSpeak NG) in $lib statically. Use: cargo --config <this file>"
        echo "[target.$triple.sherpa-onnx]"
        echo "rustc-link-search = [\"native=$lib\", \"native=$clang_rt\"]"
        printf 'rustc-link-lib = ['
        for name in sherpa-onnx-c-api sherpa-onnx-core kaldi-decoder-core sherpa-onnx-kaldifst-core \
            sherpa-onnx-fstfar sherpa-onnx-fst kaldi-native-fbank-core kissfft-float \
            piper_phonemize espeak-ng ucd onnxruntime ssentencepiece_core; do
            printf '"static=%s", ' "$name"
        done
        echo "\"static:-bundle=$rt\", \"dylib=c++\", \"framework=Foundation\", \"framework=CoreML\"]"
    } > "$config"
    echo "and for the iPad's build: cargo --config $config"
fi
