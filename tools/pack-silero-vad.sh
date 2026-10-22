#!/usr/bin/env bash
# Packs Silero VAD v5 as the archive solrac-voice downloads for listening (cjus/chessgui#134):
# target/silero-vad-v5.tar.bz2, ready to copy to the PNX Studios CDN.
#
#   tools/pack-silero-vad.sh
#
# The other models are sherpa-onnx's archives, copied unchanged. Silero VAD is published as
# a bare .onnx file, so this packs it the same way: a folder, silero-vad-v5/, holding the model
# and its licence, in a tar.bz2 that src/models.rs checks by its size and SHA-256. The model
# comes from Silero's own repository, at its v5.0 tag, and is checked against SILERO_SHA256
# before it is packed.
#
# The archive's SHA-256 depends on the files' times, so packing it again gives another; the
# one on the CDN is the one src/models.rs names. Upload it with
# `aws s3 cp target/silero-vad-v5.tar.bz2 s3://pnxstudios-solrac/solrac/models/`, and add it
# to the NOTICE.txt and SHA256SUMS beside it.
set -euo pipefail

if [[ $# -gt 0 ]]; then
    sed -n '2,16p' "$0"
    exit 2
fi
cd "$(dirname "$0")/.."

TAG="v5.0"
SILERO_URL="https://github.com/snakers4/silero-vad/raw/refs/tags/$TAG/files/silero_vad.onnx"
LICENSE_URL="https://raw.githubusercontent.com/snakers4/silero-vad/$TAG/LICENSE"
SILERO_SHA256="6b99cbfd39246b6706f98ec13c7c50c6b299181f2474fa05cbc8046acc274396"
FOLDER="silero-vad-v5"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir "$work/$FOLDER"
curl -fsSL -o "$work/$FOLDER/silero_vad.onnx" "$SILERO_URL"
curl -fsSL -o "$work/$FOLDER/LICENSE" "$LICENSE_URL"
got="$(shasum -a 256 "$work/$FOLDER/silero_vad.onnx" | cut -d' ' -f1)"
if [[ "$got" != "$SILERO_SHA256" ]]; then
    echo "silero_vad.onnx has SHA-256 $got, not $SILERO_SHA256" >&2
    exit 1
fi
cat > "$work/$FOLDER/README.txt" <<EOF
Silero VAD $TAG, by the Silero Team: silero_vad.onnx, unchanged, from
$SILERO_URL
MIT licence (LICENSE). https://github.com/snakers4/silero-vad
Packed for solrac-voice by tools/pack-silero-vad.sh.
EOF
mkdir -p target
# No extended attributes or macOS ._ files: only the three files.
COPYFILE_DISABLE=1 tar --no-xattrs -cjf "target/$FOLDER.tar.bz2" -C "$work" "$FOLDER"
size="$(wc -c < "target/$FOLDER.tar.bz2" | tr -d ' ')"
sha="$(shasum -a 256 "target/$FOLDER.tar.bz2" | cut -d' ' -f1)"
echo "target/$FOLDER.tar.bz2: $size bytes, SHA-256 $sha"
