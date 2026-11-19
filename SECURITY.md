# Security Policy

## Supported versions

LightSnap is a small hobby project. Only the latest release gets security fixes.
Please update to the [latest release](https://github.com/badger12345/lightsnap/releases/latest)
before you report an issue.

## Reporting a vulnerability

Please do not open a public issue for security problems.

Report them privately through GitHub instead:
[Report a vulnerability](https://github.com/badger12345/lightsnap/security/advisories/new).

Please include:

- What the problem is and what an attacker could do with it
- Steps to reproduce, or a proof of concept
- Your OS version, desktop (on Linux) and LightSnap version

You will get an answer within 7 days. Once a fix is released, the advisory will be
published and you will be credited, unless you prefer not to be.

## Scope

LightSnap has no network access: no uploads, accounts, telemetry or update checks.
Relevant issues are, for example, bugs that leak captures or clipboard content to other
apps or users, unsafe file handling, or problems in the bundled ffmpeg build.
