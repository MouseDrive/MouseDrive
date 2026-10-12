#!/usr/bin/env bash
set -euo pipefail

usage="usage: check-zip.sh <MouseDrive-vX.Y.Z-linux-x64.zip> <vX.Y.Z>"
zip_file=${1:?$usage}
tag=${2:?$usage}

app_id=io.github.MouseDrive.MouseDrive
top="MouseDrive-$tag"
status=0

fail() {
    echo "::error::$1" >&2
    status=1
}

entries=$(unzip -Z1 "$zip_file")

while IFS= read -r entry; do
    [[ $entry == "$top/"* ]] || fail "$entry is outside the top folder $top/"
done <<<"$entries"

for file in \
    mousedrive \
    README.md \
    LICENSE \
    CHANGELOG.md \
    THIRD_PARTY_LICENSES.html \
    packaging/README.md \
    packaging/linux/70-mousedrive.rules \
    packaging/linux/71-mousedrive-keyboard.rules \
    packaging/linux/mousedrive.conf \
    "packaging/linux/$app_id.desktop" \
    "packaging/linux/$app_id.metainfo.xml" \
    "packaging/linux/$app_id.png" \
    packaging/linux/scripts/postinst \
    packaging/linux/scripts/postrm; do
    grep -qxF "$top/$file" <<<"$entries" || fail "$zip_file has no $top/$file"
done

mode=$(unzip -Z "$zip_file" "$top/mousedrive" | awk -v name="$top/mousedrive" '$NF == name { print $1 }' || true)
[[ $mode == -rwx* ]] || fail "$top/mousedrive is not executable in $zip_file ($mode)"

exit "$status"
