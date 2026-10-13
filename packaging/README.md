# Packaging

Files for installing MouseDrive on Linux. The release zip carries this folder as `packaging/`; the `.deb` and `.rpm` packages install the same files, and so does the AUR package `mousedrive-bin` built from `aur/PKGBUILD` (not on the AUR yet).

| File | Installed as | Purpose |
|------|--------------|---------|
| `linux/70-mousedrive.rules` | `/usr/lib/udev/rules.d/` | Lets the user of the active local session use `/dev/uinput` and read mice — the same rule as the **Setup** window |
| `linux/71-mousedrive-keyboard.rules` | the package's documentation folder only (`/usr/share/doc/…`), not active | Optional keyboard access for keyboard hotkeys and gear keys. Every program in the session can then read all keystrokes; read the comments in the file first |
| `linux/mousedrive.conf` | `/usr/lib/modules-load.d/` | Loads `uinput` at boot |
| `linux/io.github.MouseDrive.MouseDrive.desktop` | `/usr/share/applications/` | Menu entry; `StartupWMClass` matches the window's app ID |
| `linux/io.github.MouseDrive.MouseDrive.png` (zip only; `MouseDrive/image/icon.png` in the source) | `/usr/share/icons/hicolor/256x256/apps/` | 256 px icon |
| `linux/io.github.MouseDrive.MouseDrive.metainfo.xml` | `/usr/share/metainfo/` | AppStream data for software centres |
| `linux/scripts/postinst`, `postrm` | package scripts | Load `uinput`, reload the udev rules and apply them to existing devices |
| `aur/PKGBUILD` | — | AUR package `mousedrive-bin` (not published yet); each release workflow writes `pkgver` and `sha256sums` into the `aur` artifact |

Packages are built without the auto-updater (`--no-default-features`): the package manager updates them.

## Installing from the zip

Run these commands in the extracted `MouseDrive-vX.Y.Z` folder (`/usr/local` keeps the files apart from the package manager's):

```sh
sudo install -Dm755 mousedrive /usr/local/bin/mousedrive
sudo install -Dm644 packaging/linux/70-mousedrive.rules /etc/udev/rules.d/70-mousedrive.rules
sudo install -Dm644 packaging/linux/mousedrive.conf /etc/modules-load.d/mousedrive.conf
sudo install -Dm644 packaging/linux/io.github.MouseDrive.MouseDrive.desktop /usr/local/share/applications/io.github.MouseDrive.MouseDrive.desktop
sudo install -Dm644 packaging/linux/io.github.MouseDrive.MouseDrive.png /usr/local/share/icons/hicolor/256x256/apps/io.github.MouseDrive.MouseDrive.png
sudo modprobe uinput
sudo udevadm control --reload-rules
sudo udevadm trigger --subsystem-match=misc --subsystem-match=input
```

## Building the packages

From `MouseDrive/`, with [cargo-deb](https://crates.io/crates/cargo-deb), [cargo-generate-rpm](https://crates.io/crates/cargo-generate-rpm) and [cargo-about](https://crates.io/crates/cargo-about) installed (cargo-about needs its `cli` feature; the font license step needs `jq`):

```sh
cargo install --locked cargo-deb cargo-generate-rpm
cargo install --locked --features cli cargo-about
cargo about generate --locked --fail --all-features -o ../THIRD_PARTY_LICENSES.html about.hbs
bash ../.github/scripts/add-font-licenses.sh ../THIRD_PARTY_LICENSES.html Cargo.toml
cargo build --release --locked --no-default-features
cargo deb --no-build --locked
cargo generate-rpm
```

## Releasing

Before pushing the tag `vX.Y.Z`, set `version = "X.Y.Z"` in `MouseDrive/Cargo.toml` and add `<release version="X.Y.Z" date="…">` as the first release in the metainfo file. The workflow stops when the tag, the package version and the metainfo disagree, when `cargo deny` finds a new advisory, or when a Linux binary needs a glibc newer than 2.35.

## Signed releases

The release workflow signs `SHA256SUMS.txt` with [minisign](https://jedisct1.github.io/minisign/) when the repository secret `MINISIGN_SECRET_KEY` is set (and `MINISIGN_PASSWORD`, if the key has a password). The trusted comment is `MouseDrive vX.Y.Z SHA256SUMS.txt`; the updater checks that it names the release it installs.

The updater only requires signatures once `RELEASE_PUBLIC_KEY` in `MouseDrive/src/update/signature.rs` holds the public key. Order matters, because every version that embeds the key rejects unsigned updates:

1. Create the key pair (`minisign -G`), keep the secret key and its password safe, and store both as repository secrets.
2. Publish a release with the secret set and check that `SHA256SUMS.txt.minisig` is attached.
3. Put the public key into `RELEASE_PUBLIC_KEY`. From then on the workflow refuses to publish a release that it cannot sign or whose signature does not match that key.
