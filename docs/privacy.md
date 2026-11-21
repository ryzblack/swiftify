# what swiftify sends, and where

swiftify has no telemetry, analytics or crash reporting, and no server of its own. checked
2026-11-21 against every host in `src/`, `qml/` and `spotify-core/src/`.

| goes to | what | when |
| --- | --- | --- |
| spotify (`api.spotify.com`, `accounts.spotify.com`, librespot's access points) | your login, library calls, streams | spotify is your service |
| youtube music (`music.youtube.com`, `www.youtube.com`, googlevideo) | searches, library calls, streams, and each song you play added to your youtube history | youtube music is your service; history only when signed in |
| stats.fm (`api.stats.fm`) | the public username you typed | only with "stats from: stats.fm" (off by default) |
| discord (local socket) | the song playing | only with rich presence on (off by default) |

kept on this computer:

- logins: spotify's and youtube music's in the keychain (secret service, keychain, credential
  manager) through QtKeychain, never in a file. youtube's is its cookies, under
  `youtube-music-login`, apart from spotify's, so logging out of one keeps the other
- settings: `swiftify.conf` in the config folder (flatpak: `~/.var/app/dev.shredos.Swiftify/config`)
- listening history for the stats: local only (`src/stats/PlayHistory`), exportable from the settings
- song caches: spotify's (encrypted, as spotify sends it) and youtube's downloaded audio
  (`<cache>/youtube`), both trimmed to the size set under audio quality

history on other devices: spotify keeps its recently played itself, and youtube music songs are
added to the account's youtube history the way youtube's own player does. there is nothing to
sync and no third party: your account carries it.

yt-dlp, which fetches youtube's audio, runs with `--no-config` so your own yt-dlp settings
(cookies, proxies, hooks) never apply; it sends nothing but the requests for the song.
