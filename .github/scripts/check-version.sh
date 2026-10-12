#!/usr/bin/env bash
set -euo pipefail

tag=${1:?usage: check-version.sh vX.Y.Z}
repo=$(cd "$(dirname "$0")/../.." && pwd)
manifest="$repo/MouseDrive/Cargo.toml"
metainfo="$repo/packaging/linux/io.github.MouseDrive.MouseDrive.metainfo.xml"

fail() {
    echo "::error::$1" >&2
    exit 1
}

[[ $tag =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "invalid tag $tag (expected vX.Y.Z)"

version=$(cargo metadata --locked --no-deps --format-version 1 --manifest-path "$manifest" |
    jq -r '.packages[] | select(.name == "mousedrive") | .version')
[[ "v$version" == "$tag" ]] || fail "tag $tag does not match the version in MouseDrive/Cargo.toml ($version)"

newest=$(grep -o '<release version="[^"]*"' "$metainfo" | head -n 1 | cut -d '"' -f 2 || true)
[[ $newest == "$version" ]] ||
    fail "the newest <release> in the AppStream metainfo is ${newest:-missing}, not $version"

echo "tag $tag matches package version $version"
