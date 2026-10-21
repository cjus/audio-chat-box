#!/usr/bin/env bash
# Makes the recordings for the transcription bake-off (cjus/chessgui#59, Solrac's
# docs/chat.md "Local transcription"): each phrase of phrases.txt spoken by six of macOS's
# voices and three of Kokoro's, as 16 kHz mono 16-bit WAV files, the format of Solrac's
# recordings. macOS only (`say` and `afconvert`). The bake-off itself is
# `cargo run --release --example bakeoff` (examples/bakeoff.rs).
#
#   tools/bakeoff/make-clips.sh <out folder> [kokoro model folder]
#
# The Kokoro folder defaults to Solrac's downloaded model. Each clip is named
# `<phrase number>-<voice>.wav`, and phrases.txt is copied beside them for the bake-off.
set -euo pipefail

out="${1:?usage: make-clips.sh <out folder> [kokoro model folder]}"
kokoro="${2:-$HOME/Library/Application Support/Solrac/models/kokoro-multi-lang-v1_0}"
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"

# British, Australian, Irish and American English.
mac_voices=("Daniel" "Karen" "Moira" "Eddy (English (US))" "Reed (English (US))" "Flo (English (US))")
kokoro_voices=(am_michael af_heart bm_george)

mkdir -p "$out"
cp "$here/phrases.txt" "$out/phrases.txt"
(cd "$root" && cargo build --release -q --example say)
say_kokoro="$root/target/release/examples/say"

number=0
while IFS='|' read -r kind spoken _expected; do
    [[ -z "$kind" || "$kind" == \#* ]] && continue
    number=$((number + 1))
    clip="$(printf '%02d' "$number")"
    for voice in "${mac_voices[@]}"; do
        name="$(echo "${voice%% *}" | tr '[:upper:]' '[:lower:]')"
        say -v "$voice" -o "$out/$clip-$name.wav" --file-format=WAVE --data-format=LEI16@16000 "$spoken"
    done
    for voice in "${kokoro_voices[@]}"; do
        # Kokoro speaks at 24 kHz in 32-bit floats: brought to Solrac's recordings' format.
        "$say_kokoro" "$kokoro" "$out/$clip-$voice.24k.wav" "$spoken" "$voice" > /dev/null
        afconvert -f WAVE -d LEI16@16000 -c 1 "$out/$clip-$voice.24k.wav" "$out/$clip-$voice.wav"
        rm "$out/$clip-$voice.24k.wav"
    done
done < "$here/phrases.txt"
echo "$(ls "$out"/*.wav | wc -l | tr -d ' ') clips in $out"
