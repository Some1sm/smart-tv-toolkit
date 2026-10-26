# Smart TV Toolkit

A desktop app for managing smart TVs from your computer: device info, apps,
performance tuning, settings and more.

Built in Rust with [iced](https://iced.rs). A rewrite of
[Android TV Desktop Toolkit](https://github.com/steven-ahfu/Android-TV-Desktop-Toolkit)
by steven-ahfu (see [NOTICE.md](NOTICE.md)).

## Platforms

| Platform | Connection | Status |
|---|---|---|
| Android TV / Google TV | ADB over the network | Supported |
| LG webOS | — | Planned |
| Samsung Tizen | — | Planned |

## Download

Get the latest build from the [Releases](../../releases) page:

- **Windows:** `SmartTVToolkit-<version>-windows-x64.zip`. Unzip and run
  `Smart TV Toolkit.exe`; nothing else to install.
- **macOS (Apple Silicon and Intel):** `SmartTVToolkit-<version>-macos-universal.dmg`.
  Open it and drag the app to Applications.

The builds aren't signed with a paid developer certificate, so the first launch
needs one extra click:

- **Windows:** if SmartScreen says "Windows protected your PC", choose
  **More info → Run anyway**.
- **macOS:** right-click the app → **Open** → **Open**. If macOS still refuses,
  run `xattr -dr com.apple.quarantine "/Applications/Smart TV Toolkit.app"`.
  macOS also asks to allow access to devices on your local network; allow it, or
  TVs can't be found.

## Requirements (Android TV)

Nothing to install: the app speaks the ADB protocol itself, so `adb.exe` and the
Android SDK aren't needed.

- On the TV: Settings → Device Preferences → About → press **Build** 7 times, then
  enable **Network debugging** / **ADB debugging** in Developer options.
- The first time you connect, the TV asks to allow debugging from this computer.
  Tick **Always allow**. The key is stored where adb keeps its own
  (`~/.android/adbkey`), so a PC that already used adb with the TV is trusted
  straight away.
- Android 11+ **Wireless debugging** (pairing code) isn't supported yet; TVs with
  classic network debugging on port 5555 work.

## Building

```bash
cargo run --release
```

```bash
cargo test
```

Building on Windows needs the Visual Studio C++ build tools (running the app doesn't). If linking fails with
`cannot open input file 'kernel32.lib'`, or with `link: extra operand` in Git Bash
(where Git's own `link` shadows Microsoft's), build from a **Developer PowerShell**
or **x64 Native Tools** prompt so the MSVC environment is set.

`cargo test -- --ignored` also runs live tests against a real TV; set
`STT_TEST_TV=ip:port` first.

### Packaging

- Windows: `.\scripts\package-windows.ps1` → `dist\SmartTVToolkit-<version>-windows-x64.zip`
- macOS: `scripts/bundle-macos.sh` → `dist/SmartTVToolkit-<version>-macos-universal.dmg`
  (needs `rustup target add aarch64-apple-darwin x86_64-apple-darwin`)
- License notices for bundled crates: `cargo about generate about.hbs -o THIRD-PARTY-LICENSES.html`
  (needs `cargo install cargo-about --features cli`); both scripts include the file when present.
- Icons: `uv run --with pillow python scripts/make_icons.py` regenerates `assets/`.

### Releasing

Bump `version` in `Cargo.toml`, commit, then push a matching tag:

```bash
git tag v0.1.0
```

```bash
git push origin v0.1.0
```

GitHub Actions builds both packages and creates a **draft** release with them
attached; check the notes and publish it. Every push to `main` also runs the
tests on Windows and macOS.

## Roadmap (Android TV parity with the original)

1. Foundation: window, sidebar and tabs, built-in ADB (no adb.exe), connect by IP ✅
2. Devices: network scan ✅, mDNS discovery ✅, device history ✅ (imports the original's
   `devices.json`), wireless pairing
3. Info tab and Health Check ✅
4. Packages ✅, Performance ✅, Install ✅, Remote ✅, Screenshot ✅, Settings ✅, Tools ✅,
   Media (scrcpy, screen recording, file browser) ✅
5. Packaging: Windows zip and universal macOS app with icons, license notices, and
   GitHub Actions for tests and releases ✅

## License

GNU Affero General Public License v3.0. See [LICENSE](LICENSE) and [NOTICE.md](NOTICE.md).
