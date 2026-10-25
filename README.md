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

- [Android SDK Platform-Tools](https://developer.android.com/tools/releases/platform-tools) (ADB).
  The app looks for it in a saved location, then `platform-tools/` or `bin/` next to
  the app, the usual SDK install folders, and `PATH`. You can also pick it from the
  sidebar (**Change…**).
- On the TV: Settings → Device Preferences → About → press **Build** 7 times, then
  enable **Network debugging** / **ADB debugging** in Developer options.

## Building

```bash
cargo run --release
```

```bash
cargo test
```

On Windows you need the Visual Studio C++ build tools. If linking fails with
`cannot open input file 'kernel32.lib'`, or with `link: extra operand` in Git Bash
(where Git's own `link` shadows Microsoft's), build from a **Developer PowerShell**
or **x64 Native Tools** prompt so the MSVC environment is set.

`cargo test -- --ignored` also runs a live test against a real TV; set
`STT_TEST_TV=ip:port` first.

## Roadmap (Android TV parity with the original)

1. Foundation: window, sidebar and tabs, ADB runner and location picker, connect by IP ✅
2. Devices: network scan, mDNS discovery, wireless pairing, device history
3. Info tab and Health Check
4. Packages, Install, Performance, Settings, Tools, Media, Remote, Files
5. Packaging: single executable, automated builds for Windows and macOS

## License

GNU Affero General Public License v3.0. See [LICENSE](LICENSE) and [NOTICE.md](NOTICE.md).
