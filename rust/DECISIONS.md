# Rust migration: decisions

The source of truth for the port. A changed decision gets a new ADR that supersedes the old one.

## ADR-001: Keep the QML interface, move the logic under it to Rust
Status: accepted (2026-11-21)
Context: the interface is 19k lines of QML and 1.7k of shaders; the C++ (42k lines in `src/`)
is the logic and the QObjects QML binds to. A Rust UI toolkit (Slint, egui) cannot reproduce the
QML scene, its animations and shaders without visible differences.
Decision: QML and shaders stay. Logic moves into the Rust core (`spotify-core`); QObjects move to
Rust through cxx-qt as their logic leaves them.
Consequences: the app stays a Qt Quick app with a Rust core; the visual design does not change.

## ADR-002: Keep the custom scene-graph items in C++
Status: accepted (2026-11-21)
Context: `src/render/*` (material shapes, spectrum, soft shadows, wavy lines) subclass
QQuickItem and build QSGNodes. cxx-qt has no safe binding for the scene graph.
Decision: they stay C++, documented as an exception.
Consequences: about 4k lines of C++ remain after the port.

## ADR-003: rust-anything only where it applies
Status: accepted (2026-11-21)
Context: rust-anything 2.1.0 (`analyze --whole-app`) parses the C++ as C: 75 files fail to
parse and 7k of 42k lines are seen; `bridge`/`record`/`diff` take C ABI units, and the
whole-app front door covers CLIs, servers and libraries, not a Qt GUI.
Decision: its plan is used for the order of the units; each unit's equivalence is held by
porting the original's own tests (C++ QtTest cases become Rust tests beside the port, as for
the web api in `spotify-core/src/music/spotify.rs`).
Consequences: no `gate` runs; `cargo test` with the ported cases and the C++ suite are the bar.

## ADR-004: One music-service interface, in Rust
Status: accepted (2026-11-21)
Context: the library talked to spotify's web api through `WebApi` (C++); YouTube Music had to
fit beside it without the library or the views knowing which service they show.
Decision: `MusicService` (`spotify-core/src/music/mod.rs`) with one implementation per service
(`spotify.rs`, the port of `WebApi.cpp` and `SpotifyJson.cpp`; `youtube.rs`). The C++ `WebApi`
is a thin carrier over `swiftify_core_service_call`; answers use the front end's Song, Album,
Artist and Playlist shapes. A new service is one impl and one arm in `music::call`.
Consequences: `SpotifyJson.cpp` is gone; the web api's QtTest cases are Rust tests now.

## ADR-005: YouTube Music through InnerTube in Rust, audio through yt-dlp
Status: accepted (2026-11-21)
Context: the user chose InnerTube in Rust (rustypipe 0.11.4). Its catalog, library and history
calls work. Its streams do not (tested 2026-11-21): the iOS/Android/VR clients are cut off after
~1 MB without an attestation token, web clients carry no stream URLs (SABR), and rustypipe's
deobfuscator no longer reads the player (upstream unchanged since April 2025). Running yt-dlp's
EJS solver in QuickJS took 9 minutes per player and still found no stream.
Decision: everything but the audio file is Rust/InnerTube; the audio is fetched by yt-dlp
(`--no-config`, node or deno for youtube's javascript), downloaded once into the song cache and
played as a local file through the existing equalizer and visualizer.
Consequences: yt-dlp (and node or deno) is a runtime dependency for YouTube playback; the
flatpak bundles both. The first play of a song waits for its download (~5-20 s); the next song
is fetched ahead, played ones come from the cache.

## ADR-006: Listening history follows the service's own account
Status: accepted (2026-11-21)
Context: history must reach other devices without a server of our own or a third party.
Decision: Spotify keeps its recently-played server-side already; YouTube Music songs are added
to the account's YouTube history as youtube's own player does (`reportPlayback`), and read back
through `recentlyPlayed`. The local history (`src/stats/PlayHistory`) stays as it was.
Consequences: no sync service; the history is the account's, on every device it signs in on.

## ADR-007: Secrets in the keychain only
Status: accepted (2026-11-21)
Decision: YouTube Music's login (its cookies) goes to the secret service through QtKeychain under
`youtube-music-login`, apart from Spotify's entries (logging out of one keeps the other). Only the
login part of rustypipe's cache is kept; rustypipe's error reports to disk are turned off.
