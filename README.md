# Smart TV Toolkit

A desktop app for managing, cleaning up and speeding up smart TVs from your
computer: apps, storage, background apps, settings, remote control and more,
with every change recorded so it can be undone.

Built in Rust with [iced](https://iced.rs), in English and Spanish. A rewrite of
[Android TV Desktop Toolkit](https://github.com/steven-ahfu/Android-TV-Desktop-Toolkit)
by steven-ahfu (see [NOTICE.md](NOTICE.md)).

## Platforms

| Platform | Connection | Status | Tabs |
|---|---|---|---|
| Android TV / Google TV | ADB over the network | Supported | All |
| Samsung Tizen (2016+) | The TV's remote-control API (ports 8001/8002) | Experimental | Info, Apps, Remote |
| LG webOS | The LG phone-remote API (ports 3000/3001) | Experimental | Info, Apps, Remote |

Samsung and LG TVs are found by the network scan like Android TVs. Their support
hasn't been tried on many real TVs yet, so reports are welcome.

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

## Requirements (Samsung and LG)

No developer mode needed. Connect to the TV's IP (the scan finds it), and accept
the prompt that appears on the TV the first time. The TV then hands out a key the
app saves, so later connections don't ask again. On older LG models, turn on
**LG Connect Apps** (or **Mobile TV On**) in the TV's network settings.

## Features (Android TV)

- **Optimize**
  - **Recommended:** safe changes applied together (disable bloatware, faster
    animations, the app freezer, trim caches, compile apps). The TV is measured
    and a snapshot saved first, so everything can be put back.
  - **TV Settings:** apps-only mode, hands-free "Hey Google", app auto-updates, the
    screen saver and the last restart, each with a button that opens the screen
    on the TV.
  - **Storage:** free space, the biggest apps with **Clear Cache** kept apart from
    **Clear Data** (logins and add-ons survive a cache clear), system app updates
    you can remove, and large files.
  - Background apps, start-at-boot apps, the home app, a cached-app limit and
    ad-blocking Private DNS.
  - **Before & After:** measure CPU, memory, swap, storage and running apps, and
    time app launches, before and after optimizing.
- **Packages:** every app with its real name and icon, a bloatware rating and
  filter (Bloatware / Optional / Keep), and batch disable, enable, uninstall, clear
  cache, clear data and force stop.
- **History:** every change is recorded with **Undo**. A snapshot of the TV's apps
  and settings is taken each day; restore one, or copy a TV's setup to others.
- **Batch TVs:** tick other TVs in the sidebar and changes are made on them too.
- **Performance:** Health Check, running processes and live CPU/RAM.
- **Settings:** a browser for the TV's settings with plain descriptions, quick
  toggles, and shortcuts that open settings screens on the TV.
- **Install:** APKs (several at once), quick-install apps with update checks, and
  Shizuku.
- **Remote, Screenshot, Media** (scrcpy mirroring and recording, a file browser,
  Play on TV), **Tools** (type text, logs, reboot) and **Wake-on-LAN**.

Good to know:

- **Packages** reads real app names from each APK and caches them. For the long
  system list, use **Load real names**.
- **Tools → Type text** handles accents and other scripts by installing the small
  ADBKeyboard helper on the TV the first time it's needed.
- **Media → scrcpy:** point the app at your scrcpy (**Locate…**) or let it download
  the latest release, checked against its published SHA-256 sums.
- Long jobs (installs, transfers, compiling) run on their own connection and show
  progress at the bottom, so the remote and other tabs keep working meanwhile.
- App data (history, snapshots, measurements, pairings) lives in
  `%APPDATA%\SmartTVToolkit` on Windows, `~/Library/Application Support/SmartTVToolkit`
  on macOS and `~/.config/SmartTVToolkit` on Linux.

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
- Linux: `scripts/package-linux.sh` → `dist/SmartTVToolkit-<version>-linux-x86_64.tar.gz`
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

GitHub Actions builds the Windows, macOS and Linux packages and creates a
**draft** release with them attached; check the notes and publish it. Every push
to `main` also runs the tests on Windows, macOS and Linux.

### Code layout

Each platform is a `Session` variant in `src/platform.rs` that lists the tabs it
supports; tabs live in `src/tabs/`, Android TV work in `src/android/`, and Samsung
and LG in `src/samsung.rs` and `src/lg.rs`. `vendor/adb_client` is a patched copy
of the adb_client crate (see its `PATCHES.md`).

## License

GNU Affero General Public License v3.0. See [LICENSE](LICENSE) and [NOTICE.md](NOTICE.md).
