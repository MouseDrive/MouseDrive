#!/usr/bin/env bash
set -euo pipefail

repo=$(cd "$(dirname "$0")/../.." && pwd)
linux="$repo/packaging/linux"
app_id=io.github.MouseDrive.MouseDrive
udev_source="$repo/MouseDrive/src/setup/udev.rs"
status=0

fail() {
    echo "::error::$1" >&2
    status=1
}

rule_lines() {
    grep -v -e '^[[:space:]]*#' -e '^[[:space:]]*$' "$1"
}

desktop-file-validate "$linux/$app_id.desktop" || fail "$app_id.desktop is not valid"
appstreamcli validate --no-net "$linux/$app_id.metainfo.xml" || fail "$app_id.metainfo.xml is not valid"

grep -qF "pub const APP_ID: &str = \"$app_id\";" "$repo/MouseDrive/src/platform.rs" ||
    fail "APP_ID in src/platform.rs is not $app_id"
grep -qx "StartupWMClass=$app_id" "$linux/$app_id.desktop" || fail "StartupWMClass is not $app_id"
grep -qx "Icon=$app_id" "$linux/$app_id.desktop" || fail "Icon is not $app_id"

for rules in 70-mousedrive.rules 71-mousedrive-keyboard.rules; do
    while IFS= read -r line; do
        grep -qF "r#\"$line\"#" "$udev_source" || fail "$rules: rule differs from src/setup/udev.rs: $line"
    done < <(rule_lines "$linux/$rules")
done
[[ $(rule_lines "$linux/70-mousedrive.rules" | wc -l) -eq 2 ]] || fail "70-mousedrive.rules must hold exactly the uinput and mouse rules"
[[ $(rule_lines "$linux/71-mousedrive-keyboard.rules" | wc -l) -eq 1 ]] || fail "71-mousedrive-keyboard.rules must hold exactly the keyboard rule"
[[ $(rule_lines "$linux/mousedrive.conf") == uinput ]] || fail "mousedrive.conf must load only uinput"

for script in "$linux"/scripts/*; do
    sh -n "$script" || fail "$script has a syntax error"
done
for script in "$repo"/.github/scripts/*.sh; do
    bash -n "$script" || fail "$script has a syntax error"
done
[[ $(grep -cxF '<!-- font-licenses -->' "$repo/MouseDrive/about.hbs" || true) -eq 1 ]] ||
    fail "MouseDrive/about.hbs must hold the font-licenses marker line exactly once"
bash -n "$repo/packaging/aur/PKGBUILD" || fail "PKGBUILD has a syntax error"
bash -n "$repo/packaging/aur/mousedrive-bin.install" || fail "mousedrive-bin.install has a syntax error"

exit "$status"
