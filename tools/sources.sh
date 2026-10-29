#!/usr/bin/env bash
# Gathers the source of the native libraries in audio-chat-box, for the "sources" release
# that every copy of the program can point to:
#
#   tools/sources.sh [<folder>]          # default: target/sources
#
# audio-chat-box as built is GPL-3.0-or-later (it links eSpeak NG), so whoever gets a copy
# may ask for the Corresponding Source: this repository at that commit, and the source of
# every library in sherpa-onnx's prebuilt static library, which no crate carries.
# tools/sources/sources.txt names each archive, where it comes from and its hash (upstream's
# wherever upstream pins it); this downloads them all, deletes any that differ, and writes
# beside them SHA256SUMS and README.md (tools/sources/README.md). Then:
#
#   gh release create sources-v<n> --draft --title "..." --notes-file <folder>/README.md <folder>/*
#
# Run it again, and make a new release, whenever the sherpa-onnx pin in Cargo.toml moves
# (licences/README.md says what else to look at then). Needs curl and sha256sum or shasum.
set -euo pipefail

if [[ $# -gt 1 || ${1:-} == -* ]]; then
    sed -n '2,19p' "$0"
    exit 2
fi
root="$(cd "$(dirname "$0")/.." && pwd)"
dir="${1:-$root/target/sources}"
mkdir -p "$dir"

# digest <algorithm> <file>: the file's hash in lowercase hex.
digest() {
    if command -v "$1sum" >/dev/null; then
        "$1sum" "$2" | cut -d ' ' -f 1
    else
        shasum -a "${1#sha}" "$2" | cut -d ' ' -f 1
    fi
}

count=0
while read -r name hash url; do
    [[ -z $name || $name == \#* ]] && continue
    algorithm=${hash%%:*}
    want=${hash#*:}
    file="$dir/$name"
    if [[ ! -f $file || $(digest "$algorithm" "$file") != "$want" ]]; then
        echo "downloading $name" >&2
        curl --fail --show-error --silent --location --retry 3 -o "$file.part" "$url"
        got=$(digest "$algorithm" "$file.part")
        if [[ $got != "$want" ]]; then
            rm -f "$file.part"
            echo "$name from $url has $algorithm $got, not $want: deleted" >&2
            exit 1
        fi
        mv "$file.part" "$file"
    fi
    count=$((count + 1))
done <"$root/tools/sources/sources.txt"

cp "$root/tools/sources/README.md" "$dir/README.md"
(
    cd "$dir"
    for file in $(grep -vE '^[[:space:]]*(#|$)' "$root/tools/sources/sources.txt" | cut -d ' ' -f 1) README.md; do
        printf '%s  %s\n' "$(digest sha256 "$file")" "$file"
    done
) >"$dir/SHA256SUMS"
echo "$count archives, README.md and SHA256SUMS in $dir" >&2
