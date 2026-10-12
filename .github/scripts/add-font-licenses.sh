#!/usr/bin/env bash
set -euo pipefail

usage="usage: add-font-licenses.sh <THIRD_PARTY_LICENSES.html> <Cargo.toml>"
html=${1:?$usage}
manifest=${2:?$usage}

marker='<!-- font-licenses -->'
crate=epaint_default_fonts

fail() {
    echo "::error::$1" >&2
    exit 1
}

escape() {
    sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g' "$1"
}

[[ $(grep -cxF "$marker" "$html" || true) -eq 1 ]] || fail "$html must hold the line $marker exactly once"

found=$(cargo metadata --locked --all-features --format-version 1 --manifest-path "$manifest" |
    jq -r --arg name "$crate" '.packages[] | select(.name == $name) | "\(.version)\t\(.manifest_path)"')
[[ -n $found && $(grep -c . <<<"$found") -eq 1 ]] || fail "expected exactly one $crate package, found: ${found:-none}"
version=${found%%$'\t'*}
fonts_dir="$(dirname "${found#*$'\t'}")/fonts"

mapfile -t texts < <(find "$fonts_dir" -maxdepth 1 -type f -name '*.txt' | sort)
((${#texts[@]} > 0)) || fail "no font license texts in $fonts_dir"

section=$(mktemp)
output=$(mktemp)
trap 'rm -f "$section" "$output"' EXIT
{
    printf '    <h2 id="fonts">Fonts bundled by egui</h2>\n'
    printf '    <p>%s %s embeds these fonts in the program. Their license texts:</p>\n' "$crate" "$version"
    for text in "${texts[@]}"; do
        printf '    <section>\n        <h3>%s</h3>\n        <pre class="license-text">' "$(basename "$text")"
        escape "$text"
        printf '</pre>\n    </section>\n'
    done
} >"$section"

awk -v marker="$marker" -v section="$section" '
    $0 == marker { while ((getline line < section) > 0) print line; next }
    { print }
' "$html" >"$output"
cat "$output" >"$html"
echo "added ${#texts[@]} font license texts of $crate $version to $html"
