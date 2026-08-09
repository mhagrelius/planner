#!/usr/bin/env bash
#
# Install the sentence-embedding model that turns on semantic duplicate search.
#
# Optional. Without it, quick-add still warns about near-identical titles using
# the word comparison, which is the whole feature for most people. With it, a
# duplicate that shares no words with what you typed — "Repair the dripping tap"
# against "Sort out the leaking tap" — is retrieved as well.
#
# It is not bundled because it is 87MB against an app whose own data file is
# tens of kilobytes, and most installs will never turn this on. Half precision
# would halve it and is not an option: f16 attention overflows to NaN on CPU,
# and is slower besides, CPU having no native f16 arithmetic.
#
#   packaging/fetch-embedding-model.sh            install
#   packaging/fetch-embedding-model.sh --remove   undo
#
# NOTE: retrieval only feeds the judgement, it does not make it. Cosine distance
# ranks reliably and cannot be thresholded — measured on real task titles, two
# unrelated errands score 0.75 simply because both are short imperative
# sentences. So this does nothing on its own: `anthropic_api_key` has to be set
# in ~/.config/planner/config.json as well, or there is nothing to adjudicate
# what the ranking turns up.

set -euo pipefail

model_dir="${XDG_DATA_HOME:-$HOME/.local/share}/planner/model"

if [[ "${1:-}" == "--remove" ]]; then
    rm -rf "$model_dir"
    echo "Removed $model_dir — semantic search is off, word comparison still on."
    exit 0
fi

# all-MiniLM-L6-v2: 22.7M parameters, 384-dimensional, the standard small
# sentence-embedding baseline. Apache 2.0.
base=https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main

# sha256 of each file as fetched on 2026-08-08, so a truncated download or a
# substituted file is caught here rather than as NaNs in a task list.
checksums="\
953f9c0d463486b10a6871cc2fd59f223b2c70184f49815e7efbcab5d8908b41  config.json
be50c3628f2bf5bb5e3a7f17b1f74611b2561a3a27eeab05e5aa30f411572037  tokenizer.json
53aa51172d142c89d9012cce15ae4d6cc0ca6895895114379cacb4fab128d9db  model.safetensors"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

for file in config.json tokenizer.json model.safetensors; do
    echo "==> $file"
    curl --fail --location --progress-bar --output "$work/$file" "$base/$file"
done

# Verified before anything is installed, so a failure here leaves whatever was
# already in place untouched rather than half-replaced.
( cd "$work" && printf '%s\n' "$checksums" | sha256sum --check --quiet ) || {
    echo "Checksums do not match — refusing to install." >&2
    echo "The upstream files may have been updated; check the model card." >&2
    exit 1
}

mkdir -p "$model_dir"
mv "$work"/{config.json,tokenizer.json,model.safetensors} "$model_dir/"

echo
echo "Installed into $model_dir"
echo "Set anthropic_api_key in ~/.config/planner/config.json to make use of it."
