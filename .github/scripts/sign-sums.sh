#!/usr/bin/env bash
set -euo pipefail

usage="usage: sign-sums.sh <SHA256SUMS.txt> <vX.Y.Z>"
sums=${1:?$usage}
tag=${2:?$usage}

repo=$(cd "$(dirname "$0")/../.." && pwd)
key_source="$repo/MouseDrive/src/update/signature.rs"
key_pattern='^const RELEASE_PUBLIC_KEY: &str = "([^"]*)";$'

key_line=$(grep -E "$key_pattern" "$key_source" || true)
if [[ $(grep -c . <<<"$key_line") -ne 1 ]]; then
    echo "::error::RELEASE_PUBLIC_KEY not found in $key_source" >&2
    exit 1
fi
[[ $key_line =~ $key_pattern ]]
public_key=${BASH_REMATCH[1]}

if [[ -z ${MINISIGN_SECRET_KEY:-} ]]; then
    if [[ -n $public_key ]]; then
        echo "::error::the app requires signed releases but the MINISIGN_SECRET_KEY secret is not set" >&2
        exit 1
    fi
    echo "::warning::MINISIGN_SECRET_KEY is not set; $sums stays unsigned"
    exit 0
fi

key_file=$(mktemp)
trap 'shred -u "$key_file" 2>/dev/null || rm -f "$key_file"' EXIT
printf '%s\n' "$MINISIGN_SECRET_KEY" >"$key_file"

comment="MouseDrive $tag SHA256SUMS.txt"
if [[ -n ${MINISIGN_PASSWORD:-} ]]; then
    printf '%s\n' "$MINISIGN_PASSWORD" | minisign -S -s "$key_file" -m "$sums" -t "$comment"
else
    minisign -S -s "$key_file" -m "$sums" -t "$comment" </dev/null
fi

if [[ -n $public_key ]]; then
    minisign -V -P "$public_key" -m "$sums"
else
    echo "::warning::$sums is signed, but RELEASE_PUBLIC_KEY is still empty, so the updater does not check the signature yet"
fi
echo "signed $sums ($comment)"
