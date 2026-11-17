#!/bin/bash
# Builds dist/LightSnap.app for Apple Silicon and a zip of it to pass around.
#
# Signing: uses the code-signing certificate named in LIGHTSNAP_SIGN_ID
# (default "LightSnap Dev") when it is in your keychain. The signature then stays
# the same from build to build, so macOS keeps the screen-recording permission.
# Without the certificate the app is signed ad hoc: it runs, but macOS asks for
# the permission again after every new build. See README.md for how to create one.
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
SIGN_ID="${LIGHTSNAP_SIGN_ID:-LightSnap Dev}"
TARGET=aarch64-apple-darwin
APP=dist/LightSnap.app

if [ ! -x build/ffmpeg/bin/ffmpeg ]; then
    ./scripts/build-ffmpeg.sh
fi

echo "==> Writing the license notices"
./scripts/notices.sh

# Keep local paths out of the binary (panic messages carry source paths). The last
# matching rule wins, so the catch-all for $HOME comes first. Encoded flags are split
# on \x1f, not on spaces, so folders with spaces in their name work.
CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
CARGO_ENCODED_RUSTFLAGS=$(printf '%s\x1f' \
    "--remap-path-prefix=$HOME=/home" \
    "--remap-path-prefix=$RUSTUP_HOME=/rustup" \
    "--remap-path-prefix=$CARGO_HOME=/cargo" \
    "--remap-path-prefix=$PWD=/lightsnap")
export CARGO_ENCODED_RUSTFLAGS="${CARGO_ENCODED_RUSTFLAGS%$'\x1f'}"

echo "==> Building LightSnap $VERSION ($TARGET)"
MACOSX_DEPLOYMENT_TARGET=13.0 cargo build --release --target "$TARGET"

echo "==> Drawing the app icon"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
cargo run --quiet --release --example app_icon -- "$WORK/AppIcon.iconset"

echo "==> Putting $APP together"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "target/$TARGET/release/lightsnap" "$APP/Contents/MacOS/lightsnap"
# Our own LGPL build (scripts/build-ffmpeg.sh), found next to the app binary at run time.
cp build/ffmpeg/bin/ffmpeg "$APP/Contents/MacOS/ffmpeg"
sed "s/@VERSION@/$VERSION/g" macos/Info.plist > "$APP/Contents/Info.plist"
plutil -lint -s "$APP/Contents/Info.plist"
iconutil -c icns "$WORK/AppIcon.iconset" -o "$APP/Contents/Resources/AppIcon.icns"
# Licenses travel with every copy of the app. The ffmpeg folder holds what the LGPL
# asks for besides the notice: our patch and the exact build script.
cp LICENSE THIRD_PARTY_NOTICES.txt "$APP/Contents/Resources/"
mkdir -p "$APP/Contents/Resources/ffmpeg"
cp macos/ffmpeg-*.patch scripts/build-ffmpeg.sh "$APP/Contents/Resources/ffmpeg/"

# Fail if a local path got into a binary.
for bin in "$APP/Contents/MacOS/"*; do
    if strings -a "$bin" | grep -qF -e "$HOME" -e "$PWD"; then
        echo "Local path found in $bin"; exit 1
    fi
done

if security find-identity -p codesigning | grep -qF "\"$SIGN_ID\""; then
    ID="$SIGN_ID"
    echo "==> Signing with \"$ID\""
else
    ID="-"
    echo "==> No certificate \"$SIGN_ID\" in the keychain: signing ad hoc"
    echo "    (macOS will ask for screen-recording permission again after each build)"
fi
# Inside out: ffmpeg first (it records the microphone, so it needs the same entitlement).
for target in "$APP/Contents/MacOS/ffmpeg" "$APP"; do
    codesign --force --options runtime --timestamp=none \
        --entitlements macos/LightSnap.entitlements --sign "$ID" "$target"
done
codesign --verify --strict "$APP"

ZIP="dist/LightSnap-$VERSION.zip"
rm -f "$ZIP"
ditto -c -k --keepParent "$APP" "$ZIP"
echo "==> Done: $APP and $ZIP"
