# LightSnap

A light screenshot and screen-recording app for the macOS menu bar and Linux desktops.
Nothing is uploaded: captures are saved locally and copied to the clipboard.

https://github.com/user-attachments/assets/b8232ba4-2def-4e8c-8b16-cafa73b4fcb8

## Requirements

Needs a Mac with Apple Silicon and macOS 27 or later, or a Linux desktop running Wayland
(see [Linux](#linux)).

## Why LightSnap

Only a few tools do both screenshots and screen recording.
Even fewer are free and open source. They also tend to get worse over time: accounts,
cloud uploads, subscriptions, ads, and more memory with every update. LightSnap is the
solution for that.

- **2 in 1:** take and annotate screenshots, and record your screen, in one tool.
- **No account, no cloud, no ads, no subscription.** LightSnap makes no network
  connections. Your captures stay on your computer.
- **Small:** a 6.4 MB download, 15 MB installed, ffmpeg included.
- **Light:** while it waits in the menu bar, it uses 0% CPU and about 100 MB of memory
  (M1 Pro, macOS 27).
- **Yours to keep:** MIT license. If the project ever goes a way you don't like, fork it.

## Install (macOS)

The app is not notarized by Apple (that needs a paid developer account), so macOS blocks it
the first time:

1. Download `LightSnap-<version>.zip` from
   [Releases](https://github.com/badger12345/lightsnap/releases), unzip it and move
   `LightSnap.app` to **Applications**.
2. Open it. macOS says it cannot check the app. Click **Done** (not "Move to Trash").
3. Go to **System Settings › Privacy & Security**, scroll down to the message about LightSnap
   and click **Open Anyway**. Confirm with your password or Touch ID.
4. Allow **Screen & System Audio Recording** when LightSnap asks, then quit and reopen it.

Instead of steps 2 and 3, you can remove the download mark in Terminal:

```bash
xattr -dr com.apple.quarantine /Applications/LightSnap.app
```

To check that your copy was built by badger12345, see [Signing](#signing).

## Build (macOS)

Needs Rust (https://rustup.rs) and the Xcode command line tools. `bundle.sh` also needs
cargo-about: `cargo install --locked --features cli cargo-about`.

```bash
./scripts/build-ffmpeg.sh  # once: build the ffmpeg that ships with the app (a few minutes)
cargo run                  # run from source
./scripts/bundle.sh        # build dist/LightSnap.app and dist/LightSnap-<version>.zip
```

Video recording, trimming and GIF export use ffmpeg. `scripts/build-ffmpeg.sh` downloads the
official source, checks its signature and builds an LGPL-only ffmpeg (no x264) that encodes
with Apple's hardware encoder. `bundle.sh` puts it into the app. When run from source,
LightSnap uses this build, or else an ffmpeg from Homebrew.

## Signing

`scripts/bundle.sh` signs the app with a certificate called **LightSnap Dev** if it is in
your keychain. Without it the app is signed ad hoc. That works too, but macOS then treats
every new build as a new app and asks for the screen-recording permission again.

The signature pins the exact certificate. macOS keeps the screen-recording permission only
as long as every build is signed with the same one. A new certificate, or a lost one, makes
every user grant the permission again. So give it a long life and keep a backup.

To create the certificate once (free, no Apple developer account):

1. Open **Keychain Access** (Spotlight: "Keychain Access", or `open -a "Keychain Access"`).
2. Menu **Keychain Access › Certificate Assistant › Create a Certificate…**
3. Name: `LightSnap Dev`, Identity Type: **Self-Signed Root**, Certificate Type: **Code Signing**.
   Tick **Let me override defaults**, then **Continue**.
4. Validity Period: `7300` days (20 years). Click **Continue** on the following pages and keep
   the defaults. Leave the email address empty, it would show in every signed app.
5. Keychain: **login**. Click **Create**.

The first build after that asks whether `codesign` may use the key: choose **Always Allow**.
To use a different certificate, set `LIGHTSNAP_SIGN_ID="<name>"`. `bundle.sh` warns 90 days
before the certificate expires and stops if two certificates have the same name.

**Backup:** In Keychain Access, under **My Certificates**, right-click **LightSnap Dev ›
Export**, save as `.p12` with a strong password, and keep the file and password in a password
manager. To restore it on another Mac, double-click the file.

Releases by badger12345 are signed with this certificate:

    LightSnap Dev, SHA-1 d51bd93470b3635b5b854c8edb609a7709b3f3a6

To check a copy, run `codesign -d -r- LightSnap.app`. It shows the SHA-1 as
`certificate leaf = H"…"`.

## Linux

LightSnap runs on Wayland desktops. X11 is not supported. It is tested on GNOME; KDE Plasma
should work too.

Wayland lets no app read the screen or catch keys on its own, so LightSnap asks the desktop
through its portals. Your desktop asks you once for each:

- **Screenshots**: full-screen and timed captures. GNOME can ask only while LightSnap's window
  is in front, so LightSnap shows a note with an **Allow…** button.
- **Screen sharing**: for recordings. Choose the screen and keep **Remember this selection**.
- **Global shortcuts**: confirm the keys. To change them later, use your desktop's settings
  (GNOME: **Settings › Apps › LightSnap**).

Capture Area and Capture Window use your desktop's own screenshot picker. While recording,
LightSnap has no window: stop or pause it from the tray icon or with the shortcut.

Needs:

- xdg-desktop-portal with a backend for your desktop (GNOME 48+ or KDE Plasma 6 for global
  shortcuts), PipeWire, and ffmpeg with x264 (`sudo apt install ffmpeg`).
- GNOME shows the tray icon with the AppIndicator extension (Ubuntu has it; on Fedora install
  "AppIndicator and KStatusNotifierItem Support").

Install: download `LightSnap-<version>-linux-<arch>.tar.gz` from
[Releases](https://github.com/badger12345/lightsnap/releases), unpack it anywhere and start
`lightsnap`. On first start it adds itself to your app list (`~/.local/share/applications`).

Build from source (Ubuntu/Debian package names):

```bash
sudo apt install build-essential pkg-config clang libclang-dev libpipewire-0.3-dev \
  libspa-0.2-dev libwayland-dev libxkbcommon-dev libdbus-1-dev ffmpeg
cargo run                      # run from source
./scripts/bundle-linux.sh      # build dist/LightSnap-<version>-linux-<arch>.tar.gz
```

The GitHub workflow `.github/workflows/linux.yml` builds the packages for x86_64 and arm64.

## Disclaimer

LightSnap is a free, non-commercial hobby project under the MIT License.
It is provided "as is", without warranty of any kind. Use it at your own risk.
To the extent permitted by law, the author is not liable for any damage or loss,
including lost data, arising from the use of or inability to use this software.
There is no support and no promise of updates or fixes.

You are responsible for how you use it. Recording other people's screens,
calls or voices may require their consent where you live.

## License

MIT, see [LICENSE](LICENSE).

The app also ships ffmpeg (LGPL 2.1+) and the code of many Rust crates. Their licenses, the
ffmpeg source, its build options and our patch are listed in
[THIRD_PARTY_NOTICES.txt](THIRD_PARTY_NOTICES.txt). `scripts/notices.sh` writes that file from
`Cargo.lock` and the ffmpeg build; `bundle.sh` runs it and copies it, `LICENSE`, the patch and
the ffmpeg build script into `LightSnap.app/Contents/Resources/`.

On Linux, ffmpeg is not shipped. `scripts/bundle-linux.sh` writes the crate licenses for the
Linux build into the package as `THIRD_PARTY_NOTICES.txt`.

Check the dependencies for known security issues with `cargo audit`.
