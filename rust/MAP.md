# Rust migration map

Goal: Swiftify's logic in Rust under the unchanged QML interface (DECISIONS.md ADR-001), with
Spotify and YouTube Music behind one service interface. Read DECISIONS.md first.

## Done

| Unit | Was | Now | Proof |
| --- | --- | --- | --- |
| spotify web api | `src/spotify/WebApi.cpp`, `SpotifyJson.cpp` (739 lines C++) | `spotify-core/src/music/spotify.rs` | Rust tests ported from tst_session's web api cases; C++ `WebApi` is a carrier |
| music-service interface | — | `spotify-core/src/music/mod.rs` | tests in the module |
| YouTube Music | — | `spotify-core/src/music/youtube.rs`, routing in `lib.rs`/`capi.rs` | unit tests; `live_search_album_and_audio_decode` against youtube |
| core playlist/top-track json for the library | `json::wrappedTrack`/`track` in C++ | `swiftify_spotify_songs_json` | tst_session playlist and top tracks cases |

Already Rust before this work: playback, DSP/equalizer, spectrum, local files and tags
(`spotify-core`, 8k lines).

## Open, in order (C++ lines as of 2026-11-21)

Each moves its logic into `spotify-core` and its QObject to cxx-qt, keeping its QtTest cases.

1. `src/stats` (2,239): PlayHistory, StatsAggregator, StatsFmClient: pure logic, few Qt types.
2. `src/library` (4,273): LocalLibrary, SongListCache, LibraryStore. SongListModel stays a
   QAbstractListModel until cxx-qt list models cover its roles.
3. `src/spotify` rest (1,500): OAuthFlow, CredentialStore (keyring crate), SpotifySession.
4. `src/playback` (2,620): PlaybackController, Lyrics, PlaybackState.
5. `src/artwork` (1,736), `src/theme` (1,967): colour and palette maths first.
6. `src/app`, `src/system`, `src/soloist`, `src/motion`, `src/core` (≈7k): integration last.

Stays C++: `src/render` (ADR-002), `third_party/material-color-utilities` (vendored).

## Out of scope

The macOS/iOS Swift front end (`swift_ffi.rs`) keeps working on the core; YouTube Music is
wired for the Qt app only.
