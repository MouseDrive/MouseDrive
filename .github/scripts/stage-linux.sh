#!/usr/bin/env bash
set -euo pipefail

usage="usage: stage-linux.sh <vX.Y.Z> <binary> <THIRD_PARTY_LICENSES.html> <out-dir>"
tag=${1:?$usage}
binary=${2:?$usage}
licenses=${3:?$usage}
out=${4:?$usage}

repo=$(cd "$(dirname "$0")/../.." && pwd)
app_id=io.github.MouseDrive.MouseDrive
top="MouseDrive-$tag"
root="$out/$top"
zip_name="MouseDrive-$tag-linux-x64.zip"

if [[ -e $root || -e "$out/$zip_name" ]]; then
    echo "::error::$root or $out/$zip_name already exists" >&2
    exit 1
fi

mkdir -p "$root/packaging"
install -m 755 "$binary" "$root/mousedrive"
install -m 644 "$repo/README.md" "$repo/LICENSE" "$repo/CHANGELOG.md" "$root/"
install -m 644 "$licenses" "$root/THIRD_PARTY_LICENSES.html"
install -m 644 "$repo/packaging/README.md" "$root/packaging/"
cp -R "$repo/packaging/linux" "$root/packaging/"
install -m 644 "$repo/MouseDrive/image/icon.png" "$root/packaging/linux/$app_id.png"

(cd "$out" && zip -q -r -X "$zip_name" "$top")
echo "$out/$zip_name"
