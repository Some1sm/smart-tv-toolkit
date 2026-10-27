# Patched copy of adb_client 3.2.3

Upstream: https://github.com/cocool97/adb_client (MIT, Copyright (c) Corentin LIAUD;
see LICENSE). Used through `[patch.crates-io]` in the app's `Cargo.toml`.

Some TVs send a session's closing messages late (after the library's 20 ms
drain), especially under load. The next command then read that leftover
message as its own answer and failed with "Open session failed: got CLSE in
response instead of OKAY" or "responses used … for our local_id instead of …".

Changes, each marked `[smart-tv-toolkit patch]`:

1. `adb_message_device.rs` (`open_session`): skip messages for other sessions
   until the answer to our OPEN arrives.
2. `adb_session.rs` (`recv_and_reply_okay`): ignore messages for other
   sessions instead of mixing them into this session's output.
3. `tcp/tcp_transport.rs` (`read_message_with_timeout`): once a message has
   started arriving, read the rest with a 10 s timeout (a short drain timeout
   could otherwise leave half a message behind), and stop on end of stream
   instead of looping.
