# codex-gui

Opensource desktop GUI app for codex. No electron, built with `gpui`.

<p align="center">
  <img width="49%" alt="image" src="https://github.com/user-attachments/assets/baedcd2b-8c84-4675-b19e-7c70229d9ebd" />
  <img width="49%" alt="image" src="https://github.com/user-attachments/assets/0b7ddc63-6b83-48fd-928d-6a46ef20fec4" />
</p>

> The frame rate is actually steady 60fps when using. HUD shows instable frame interval because there were no user interaction at that time, so no new frames were generated.


https://github.com/user-attachments/assets/75e13e95-1d42-4869-a9c0-9a102414f130

## Installation

You can download build artifacts from github action runs.


## Run

```sh
nix develop
cargo build --bin codex-code-mode-host
cargo run --bin codex-gui
```

The local code-mode host keeps Codex's upstream transport and session protocol,
but executes JavaScript with the patched QuickJS runtime in
`crates/codex-code-mode-runtime-quickjs`.

To run the same GUI against the Codex-compatible app-server in a DeepSeek
Harness source checkout:

```sh
cargo run --bin dsh-gui -- \
  --dsh-repo /path/to/deepseek-harness \
  --profile app-server
```

Without `--dsh-repo`, `dsh-gui` launches `dsh --profile app-server` from
`PATH`. The GUI continues to use the pinned Codex protocol types; the dsh
process owns model, permission, session, and configuration compatibility.

## Packaging

The flake is the canonical package definition for Linux and macOS. It also
declares the public GHCR-backed binary cache, so these commands can reuse CI
builds:

```sh
nix run --accept-flake-config .
nix build --accept-flake-config .#codex-gui
```

Linux additionally exposes `.#deb`, `.#rpm`, and `.#appimage`. macOS exposes
`.#macos-app` and `.#macos-archive`. Every package keeps `codex-gui` and
`codex-code-mode-host` in the same executable directory because the host is a
runtime component, not a standalone application.

The GitHub Actions build publishes x86_64 and aarch64 Linux DEB, RPM, and
AppImage artifacts, an aarch64 macOS app archive, and native x86_64 and
aarch64 Windows ZIP archives.
