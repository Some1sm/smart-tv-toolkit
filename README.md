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
- **Linux (x86_64):** `SmartTVToolkit-<version>-linux-x86_64.tar.gz`. Unpack it and run
  `./install.sh` to add it to your app menu, or run `./smart-tv-toolkit` directly.

The app checks GitHub once a day and shows a note in the sidebar when a newer
version is out.

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
- TVs that only offer Android 11+ **Wireless debugging**: open *Pair device with
  pairing code* on the TV, then use **Pair with a code** in the sidebar with the
  IP:port and six digits it shows. After pairing, connect with the port shown under
  *IP address & port* (the app fills it in when the TV announces it).
- The app remembers the last TV and reconnects on launch, and reconnects by itself
  if the connection drops (e.g. after the TV sleeps).

## Features worth knowing

- **Packages** shows real app names (read from each APK and cached), not just
  package ids. For the long system list, use **Load real names**.
- **Tools → Type text** handles accents and other scripts by installing the small
  ADBKeyboard helper on the TV the first time it's needed.
- **Media → scrcpy:** point the app at your scrcpy (**Locate…**) or let it download
  the latest release, checked against its published SHA-256 sums.
- Long jobs (installs, transfers, compiling) run on their own connection and show
  progress at the bottom, so the remote and other tabs keep working meanwhile.

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
   `devices.json`), Android 11+ pairing ✅
3. Info tab and Health Check ✅
4. Packages ✅, Performance ✅, Install ✅, Remote ✅, Screenshot ✅, Settings ✅, Tools ✅,
   Media (scrcpy, screen recording, file browser) ✅
5. Packaging: Windows zip, universal macOS app and Linux tarball with icons, license
   notices, and GitHub Actions for tests and releases ✅

Next: LG webOS and Samsung Tizen. Each platform is a `Session` variant in
`src/platform.rs` that lists the tabs it supports; tabs live in `src/tabs/`.

## License

GNU Affero General Public License v3.0. See [LICENSE](LICENSE) and [NOTICE.md](NOTICE.md).
