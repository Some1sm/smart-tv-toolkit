# Smart TV Toolkit

A desktop app for managing smart TVs from your computer: device info, apps,
performance tuning, settings and more.

Built in Rust with [iced](https://iced.rs). A rewrite of
[Android TV Desktop Toolkit](https://github.com/steven-ahfu/Android-TV-Desktop-Toolkit)
by steven-ahfu (see [NOTICE.md](NOTICE.md)).

## Platforms

| Platform | Connection | Status |
|---|---|---|
| Android TV / Google TV | ADB over the network | In progress |
| LG webOS | — | Planned |
| Samsung Tizen | — | Planned |

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

`cargo test -- --ignored` also runs a live test against a real TV; set
`STT_TEST_TV=ip:port` first.

## Roadmap (Android TV parity with the original)

1. Foundation: window, sidebar and tabs, built-in ADB (no adb.exe), connect by IP ✅
2. Devices: network scan ✅, mDNS discovery, wireless pairing, device history
3. Info tab and Health Check
4. Packages, Install, Performance, Settings, Tools, Media, Remote, Files
5. Packaging: single executable, automated builds for Windows and macOS

## License

GNU Affero General Public License v3.0. See [LICENSE](LICENSE) and [NOTICE.md](NOTICE.md).
