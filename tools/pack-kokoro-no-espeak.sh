#!/usr/bin/env bash
# Packs sherpa-onnx's Kokoro archive without eSpeak NG's data, for the no-eSpeak build
# (tools/no-espeak/README.md): target/models-no-espeak/kokoro-multi-lang-v1_0-no-espeak.tar.bz2,
# and the int8 one beside it, ready to copy to a product's own models folder (for Solrac, the
# PNX Studios CDN).
#
#   tools/pack-kokoro-no-espeak.sh <kokoro-multi-lang-v1_0.tar.bz2> <kokoro-int8-multi-lang-v1_0.tar.bz2>
#
# The inputs are sherpa-onnx's archives, the ones src/models.rs names in the usual build; each
# is checked against the SHA-256 below before it is used. What changes: `espeak-ng-data/`
# (GPL-3.0-or-later) is left out, the folder is renamed to match the archive (`<name>-no-espeak`,
# which is what src/models.rs looks for), and nothing else is touched. Without eSpeak NG's
# data, an archive that has it can't be mistaken for one that hasn't.
#
# The archive's SHA-256 depends on the packing, so packing again can give another: the one
# src/models.rs names (with `--features no-espeak`) is the one on the CDN. Files get one fixed
# time, to keep it from changing needlessly.
set -euo pipefail

if [[ $# -ne 2 || ${1:-} == -* ]]; then
    sed -n '2,16p' "$0"
    exit 2
fi
cd "$(dirname "$0")/.."
out=target/models-no-espeak
mkdir -p "$out"

sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }

pack() { # <archive> <sha256 of the archive> <folder>
    local archive=$1 want=$2 folder=$3
    if [[ $(sha256 "$archive") != "$want" ]]; then
        echo "$archive is not the archive sherpa-onnx published ($want): refusing to pack it" >&2
        exit 1
    fi
    local work
    work=$(mktemp -d)
    tar -xjf "$archive" -C "$work"
    [[ -d "$work/$folder" ]] || { echo "no $folder/ in $archive" >&2; exit 1; }
    rm -rf "$work/$folder/espeak-ng-data"
    mv "$work/$folder" "$work/$folder-no-espeak"
    find "$work" -exec touch -t 202611120000 {} +
    COPYFILE_DISABLE=1 tar -cjf "$out/$folder-no-espeak.tar.bz2" -C "$work" "$folder-no-espeak"
    rm -rf "$work"
    echo "$(sha256 "$out/$folder-no-espeak.tar.bz2")  $(wc -c < "$out/$folder-no-espeak.tar.bz2" | tr -d ' ')  $out/$folder-no-espeak.tar.bz2"
}

pack "$1" c5f7e2d2caf082bc1d20fb70334a61d99d20b484500aad32e7cf84c128ea3298 kokoro-multi-lang-v1_0
pack "$2" 4c3052abaa60943a341f193888cf6abd68787dae6ab8ae5c925a706caa247e4e kokoro-int8-multi-lang-v1_0
