# Contributing to LightSnap

Thanks for helping. LightSnap is a small hobby project, so please read this first.

## What fits

LightSnap does one job and stays small. Changes that keep it that way are welcome:
bug fixes, better captures and recordings, editor tools, Linux desktop support, docs.

These will not be merged:

- Network access of any kind: uploads, accounts, telemetry, update checks, crash reports.
- Large new dependencies for small features.
- X11 support. LightSnap on Linux is Wayland only.
- Code under GPL or other licenses that would stop the app from staying MIT
  (for example x264 in the macOS ffmpeg build).

## Issues

- **Bug:** say what you did, what happened and what you expected. Add your OS version,
  desktop (on Linux) and LightSnap version (Preferences › About). Screenshots help.
- **Idea:** describe the problem first, then your solution.
- **Before a larger change**, open an issue so we can agree on the approach. That saves
  you work on a pull request that cannot be merged.

## Pull requests

1. Fork the repo and create a branch from `main`.
2. Build and run it as described in the [README](README.md#build-macos). On Linux, see
   [Linux](README.md#linux).
3. Keep the change focused: one fix or feature per pull request.
4. Run `cargo test` and try your change in the running app.
5. Describe what you changed and how you tested it. For UI changes, add a screenshot.

`main` is protected. All changes go through a pull request.

### Rules for code

- Linux code lives in `src/platform/linux/` behind `cfg(target_os = "linux")`. Keep
  platform code out of shared modules.
- New crates must use a license listed in [about.toml](about.toml). `scripts/notices.sh`
  fails otherwise.
- No local paths (like `/Users/<name>/...` or `/home/<name>/...`) in code, scripts or docs.
  `scripts/bundle.sh` checks the macOS build for them.
- Do not commit build output (`target/`, `dist/`, `build/`).

## License

LightSnap is under the [MIT License](LICENSE). By sending a pull request, you agree that
your contribution is published under the same license.

## No promises

This is a free-time project. Reviews may take a while, and not every pull request will be
merged. That is not a judgment of your work.
