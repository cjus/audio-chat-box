#!/usr/bin/env bash
# Writes the third-party notices that go with every copy of the audio-chat-box program, into
# target/licences/THIRD-PARTY-NOTICES.txt.
#
#   tools/notices.sh
#
# The licences of what audio-chat-box is built with ask that their notices go with every copy:
# the MIT and BSD licences' copyright lines, Apache-2.0's NOTICE files. This gathers them:
#
# - every crate, with cargo-about (tools/notices/about.toml and the template
#   tools/notices/notices.hbs), which reads each crate's own licence files;
# - any crate's NOTICE file (none today), found through a second template,
#   tools/notices/crates.hbs, which lists the crates' folders;
# - the C and C++ libraries inside sherpa-onnx's prebuilt library, whose licences no crate
#   carries: licences/, which says where each came from (licences/README.md).
#
# A product that ships audio-chat-box beside it, as Solrac does (cjus/chessgui#98), takes
# this file with the program. Needs cargo-about 0.9 (cargo install --locked cargo-about@0.9.2 --features cli)
# and the network: it fetches, from a crate's repository, the licence files the crate didn't
# package.
set -euo pipefail

if [[ $# -gt 0 ]]; then
    sed -n '2,21p' "$0"
    exit 2
fi
cd "$(dirname "$0")/.."
if ! cargo about --version >/dev/null 2>&1; then
    echo "cargo-about isn't installed: cargo install --locked cargo-about@0.9.2 --features cli" >&2
    exit 1
fi

out="${CARGO_TARGET_DIR:-target}/licences"
notices="$out/THIRD-PARTY-NOTICES.txt"
crates=$(mktemp)
folders=$(mktemp)
trap 'rm -f "$crates" "$folders"' EXIT
mkdir -p "$out"

cargo about generate --locked -c tools/notices/about.toml -o "$crates" tools/notices/notices.hbs
cargo about generate --locked -c tools/notices/about.toml -o "$folders" tools/notices/crates.hbs

rule="--------------------------------------------------------------------------------"
{
    cat "$crates"
    shopt -s nullglob nocaseglob
    while read -r manifest; do
        [[ -n "$manifest" ]] || continue
        folder=$(dirname "$manifest")
        for notice in "$folder"/NOTICE*; do
            printf '\n%s\nThe NOTICE file of %s\n\n' "$rule" "$(basename "$folder")"
            cat "$notice"
        done
    done < "$folders"
    shopt -u nullglob nocaseglob
    printf '\n\nC and C++ libraries in sherpa-onnx'"'"'s prebuilt library\n'
    printf -- '----------------------------------------------------\n'
    for library in licences/*/; do
        for file in "$library"*; do
            printf '\n%s\n%s: %s\n\n' "$rule" "$(basename "$library")" "$(basename "$file")"
            cat "$file"
        done
    done
} > "$notices"
echo "Wrote $notices"
