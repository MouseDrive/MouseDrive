#!/usr/bin/env bash
set -euo pipefail

usage="usage: render-pkgbuild.sh <vX.Y.Z> <SHA256SUMS.txt> <out-dir>"
tag=${1:?$usage}
sums=${2:?$usage}
out=${3:?$usage}

repo=$(cd "$(dirname "$0")/../.." && pwd)
zip_name="MouseDrive-$tag-linux-x64.zip"

sum=$(awk -v name="$zip_name" '$2 == name || $2 == "*" name { print $1 }' "$sums")
if ! [[ $sum =~ ^[0-9a-f]{64}$ ]]; then
    echo "::error::no SHA-256 for $zip_name in $sums" >&2
    exit 1
fi

mkdir -p "$out"
sed -e "s/^pkgver=.*/pkgver=${tag#v}/" \
    -e "s/^pkgrel=.*/pkgrel=1/" \
    -e "s/^sha256sums=.*/sha256sums=('$sum')/" \
    "$repo/packaging/aur/PKGBUILD" >"$out/PKGBUILD"
install -m 644 "$repo/packaging/aur/mousedrive-bin.install" "$out/"

grep -qx "pkgver=${tag#v}" "$out/PKGBUILD"
grep -qx "sha256sums=('$sum')" "$out/PKGBUILD"
echo "$out/PKGBUILD"
