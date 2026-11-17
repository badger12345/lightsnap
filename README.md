# LightSnap

A light screenshot and screen-recording app for the macOS menu bar. Nothing is uploaded:
captures are saved locally and copied to the clipboard.

Runs on Apple Silicon Macs. Tested on macOS 27.

## Build

Needs Rust (https://rustup.rs). Video recording, trimming and GIF export also need ffmpeg
(for now: `brew install ffmpeg`).

```bash
cargo run                  # run from source
./scripts/bundle.sh        # build dist/LightSnap.app and dist/LightSnap-<version>.zip
```

## Signing

`scripts/bundle.sh` signs the app with a certificate called **LightSnap Dev** if it is in
your keychain. Without it the app is signed ad hoc. That works too, but macOS then treats
every new build as a new app and asks for the screen-recording permission again.

To create the certificate once (free, no Apple developer account):

1. Open **Keychain Access**.
2. Menu **Keychain Access › Certificate Assistant › Create a Certificate…**
3. Name: `LightSnap Dev`, Identity Type: **Self-Signed Root**, Certificate Type: **Code Signing**.
4. Click **Create**.

The first build after that asks whether `codesign` may use the key: choose **Always Allow**.
To use a different certificate, set `LIGHTSNAP_SIGN_ID="<name>"`.

## First start on another Mac

The app is not notarized by Apple, so macOS blocks it the first time. Open it once, then go to
**System Settings › Privacy & Security** and click **Open Anyway**. After that, allow
**Screen & System Audio Recording** for LightSnap when asked.

## License

MIT, see [LICENSE](LICENSE).
