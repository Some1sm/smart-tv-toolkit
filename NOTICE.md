# Notice

Smart TV Toolkit is a Rust rewrite of, and is derived from,
[Android TV Desktop Toolkit](https://github.com/steven-ahfu/Android-TV-Desktop-Toolkit)
by steven-ahfu, which is licensed under the GNU Affero General Public License v3.0.

Copyright (C) 2026 steven-ahfu (original Android TV Desktop Toolkit)
Copyright (C) 2026 Some1sm (Smart TV Toolkit)

## Modifications

- 2026-10-25: Rewritten from Python/CustomTkinter to Rust with the iced GUI
  framework, renamed Smart TV Toolkit, and restructured so that other TV
  platforms (LG webOS, Samsung Tizen) can be added alongside Android TV.

This program is free software: you can redistribute it and/or modify it under
the terms of the GNU Affero General Public License as published by the Free
Software Foundation, version 3. It is distributed WITHOUT ANY WARRANTY; see
[LICENSE](LICENSE) for details.

## Third-party components

- [iced](https://iced.rs) and the other Rust crates in `Cargo.toml` are used
  under their own licenses (mostly MIT and/or Apache-2.0).
- The ADB protocol is implemented by the [adb_client](https://github.com/cocool97/adb_client)
  crate (MIT). No Android SDK Platform-Tools binaries are included or required.
