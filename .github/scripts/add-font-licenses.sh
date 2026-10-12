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

metadata=$(cargo metadata --locked --all-features --format-version 1 --manifest-path "$manifest")
package_info() {
    jq -r --arg name "$1" '.packages[] | select(.name == $name) | "\(.version)\t\(.manifest_path)"' <<<"$metadata"
}

found=$(package_info "$crate")
[[ -n $found && $(grep -c . <<<"$found") -eq 1 ]] || fail "expected exactly one $crate package, found: ${found:-none}"
version=${found%%$'\t'*}
fonts_dir="$(dirname "${found#*$'\t'}")/fonts"

cantarell_crate=sctk-adwaita
cantarell_notices=(
    'Copyright (c) 2009-2011, Understanding Limited (dave@understandinglimited.com),'
    'Copyright (c) 2010-2011, Jakub Steiner (jimmac@gmail.com).'
    'This Font Software is licensed under the SIL Open Font License, Version 1.1.'
)
cantarell_found=$(package_info "$cantarell_crate")
cantarell_font=
if [[ -n $cantarell_found ]]; then
    [[ $(grep -c . <<<"$cantarell_found") -eq 1 ]] || fail "expected exactly one $cantarell_crate package, found: $cantarell_found"
    cantarell_font="$(dirname "${cantarell_found#*$'\t'}")/src/title/Cantarell-Regular.ttf"
    [[ -f $cantarell_font ]] || fail "$cantarell_crate no longer ships $cantarell_font; update this script"
    for notice in "${cantarell_notices[@]}"; do
        grep -aqF "$notice" "$cantarell_font" || fail "$cantarell_font no longer carries \"$notice\"; update this script"
    done
fi

mapfile -t texts < <(find "$fonts_dir" -maxdepth 1 -type f -name '*.txt' | sort)
((${#texts[@]} > 0)) || fail "no font license texts in $fonts_dir"
[[ -z $cantarell_font || -f $fonts_dir/OFL.txt ]] || fail "$fonts_dir/OFL.txt is missing, but the Cantarell notice refers to it"

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
    if [[ -n $cantarell_font ]]; then
        printf '    <h2 id="cantarell">Cantarell font in Wayland window decorations</h2>\n'
        printf '    <p>%s %s embeds the Cantarell Regular font for the title bars that MouseDrive draws itself on Wayland desktops without server-side decorations (for example GNOME).</p>\n' "$cantarell_crate" "${cantarell_found%%$'\t'*}"
        printf '    <section>\n        <h3>Cantarell-Regular.ttf</h3>\n        <pre class="license-text">'
        printf '%s\n' "${cantarell_notices[@]:0:2}"
        printf '\n%s The license text is the one printed under OFL.txt above.' "${cantarell_notices[2]}"
        printf '</pre>\n    </section>\n'
    fi
} >"$section"

awk -v marker="$marker" -v section="$section" '
    $0 == marker { while ((getline line < section) > 0) print line; next }
    { print }
' "$html" >"$output"
cat "$output" >"$html"
echo "added ${#texts[@]} font license texts of $crate $version${cantarell_font:+ and the Cantarell notice} to $html"
