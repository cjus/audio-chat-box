#!/usr/bin/env bash
# Downloads models into a folder as audio-chat-box would, for an image that carries them (the
# Dockerfile's demo image):
#
#   tools/fetch-models.sh <models folder> <model id>...
#
# Each archive comes from where src/models.rs says (MODELS_URL, this repository's models-v1
# release) and must have the size and SHA-256 that src/models.rs gives it, or it is deleted
# without being unpacked. src/models.rs stays the one place those are written: this reads them
# from it. A transcription model also gets the bpe.vocab that audio-chat-box writes into its
# folder when it loads (src/hear.rs, `vocabulary`), so the folder can then be read-only: the
# program writes that file only when it is missing or different. If that rule changes, change
# it here too. Needs curl, bzip2, tar and sha256sum.
set -euo pipefail

if [[ $# -lt 2 ]]; then
    sed -n '2,13p' "$0"
    exit 2
fi
models_rs="$(cd "$(dirname "$0")/.." && pwd)/src/models.rs"
dir="$1"
shift

base_url=$(sed -n 's/^pub const MODELS_URL: &str = "\(.*\)";$/\1/p' "$models_rs")
if [[ -z $base_url ]]; then
    echo "MODELS_URL isn't in $models_rs" >&2
    exit 1
fi

# field <model id> <name>: the value of the field <name> of the LocalModel whose id is <model id>.
field() {
    awk -v id="$1" -v name="$2" '
        /= LocalModel \{$/ { inside = 1; found = 0; value = ""; next }
        inside && /^};$/ { if (found) { print value; exit } inside = 0; next }
        inside && match($0, /^    [a-z0-9_]+: /) {
            key = substr($0, 5, RLENGTH - 6)
            val = substr($0, RLENGTH + 1)
            sub(/,$/, "", val)
            gsub(/"/, "", val)
            if (key == "id" && val == id) found = 1
            if (key == name) value = val
        }
    ' "$models_rs"
}

mkdir -p "$dir"
for id in "$@"; do
    archive=$(field "$id" archive)
    bytes=$(field "$id" archive_bytes)
    bytes=${bytes//_/}
    sha256=$(field "$id" sha256)
    folder=$(field "$id" folder)
    model_file=$(field "$id" model_file)
    purpose=$(field "$id" purpose)
    if [[ -z $archive || -z $bytes || -z $sha256 || -z $folder || -z $model_file ]]; then
        echo "src/models.rs has no model $id" >&2
        exit 1
    fi

    part="$dir/$archive.part"
    echo "$id: downloading $archive ($bytes bytes)" >&2
    curl --fail --show-error --silent --location --retry 3 --max-filesize "$bytes" \
        -o "$part" "$base_url$archive"
    got_bytes=$(wc -c <"$part" | tr -d ' ')
    got_sha256=$(sha256sum "$part" | cut -d ' ' -f 1)
    if [[ $got_bytes != "$bytes" || $got_sha256 != "$sha256" ]]; then
        rm -f "$part"
        echo "$archive isn't the archive src/models.rs names ($got_bytes bytes, SHA-256" \
            "$got_sha256): deleted, not unpacked" >&2
        exit 1
    fi

    staging="$dir/.unpacking-$folder"
    rm -rf "$staging"
    mkdir "$staging"
    tar -xjf "$part" -C "$staging"
    rm -f "$part"
    if [[ ! -f $staging/$folder/$model_file ]]; then
        echo "$archive doesn't hold $folder/$model_file" >&2
        exit 1
    fi
    if [[ $purpose == *Transcribe ]]; then
        # Each line of tokens.txt is a piece, a space and its number; bpe.vocab lists the
        # pieces but the empty, <unk> and <blk> ones, each with a score of -1.
        LC_ALL=C awk '{
            i = match($0, / [^ ]*$/)
            if (i) {
                piece = substr($0, 1, i - 1)
                if (piece != "" && piece != "<unk>" && piece != "<blk>") printf "%s\t-1\n", piece
            }
        }' "$staging/$folder/tokens.txt" >"$staging/$folder/bpe.vocab"
    fi
    rm -rf "${dir:?}/$folder"
    mv "$staging/$folder" "$dir/$folder"
    rm -rf "$staging"
    echo "$id: unpacked into $dir/$folder" >&2
done
