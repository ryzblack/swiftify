# swiftify as a flatpak

## build and install

needs `flatpak` and `flatpak-builder`, about 6 GB free the first time (the KDE SDK, rust and
node extensions, and the build), and internet for the sources.

```sh
packaging/flatpak/build.sh            # builds and installs dev.shredos.Swiftify for this user
packaging/flatpak/build.sh --bundle   # also writes swiftify.flatpak to install elsewhere
flatpak run dev.shredos.Swiftify
```

a bundle installs on another machine with `flatpak install --user swiftify.flatpak` (it pulls
the KDE runtime from flathub). the manifest is `dev.shredos.Swiftify.yml`; the crates come from
`cargo-sources.json`, which `build.sh` regenerates from `spotify-core/Cargo.lock` when the lock
is newer (`cargo-sources.py`, stdlib python only).

## what the sandbox allows, and why

| permission | for |
| --- | --- |
| network | streaming, and spotify's login callback on localhost |
| wayland, fallback x11, dri, ipc | the window |
| pulseaudio | the audio output and following the default device |
| `org.freedesktop.secrets` | the logins, in your keychain |
| own `org.mpris.MediaPlayer2.swiftify` | media keys and the desktop's player controls |
| system `org.freedesktop.login1` | pausing for sleep |
| `xdg-music` | local files and editing their tags; other folders through the file chooser |
| discord's ipc socket | rich presence, off unless turned on |

no home folder access. a keychain must run (gnome-keyring, kwallet, keepassxc) as for the
native build.

## data and updates

settings, caches and the local history live in `~/.var/app/dev.shredos.Swiftify/`, which
`flatpak update` leaves alone, so updates keep everything. logins are in the keychain and are
shared with a native install. to bring a native install's settings over once:

```sh
mkdir -p ~/.var/app/dev.shredos.Swiftify/config
cp -r ~/.config/Swiftify ~/.var/app/dev.shredos.Swiftify/config/
```

## spotify and youtube music inside it

both work as in the native build: spotify through librespot, youtube music through its api, its
audio straight from youtube music's player in the rust core. nothing outside the app is run, and
no system program is needed.

## .flatpakref

a `.flatpakref` points at a hosted repo, which swiftify does not run. to publish one, serve
`flatpak-repo/` (built by `build.sh`) over https, sign it with `--gpg-sign`, and write:

```ini
[Flatpak Ref]
Name=dev.shredos.Swiftify
Branch=master
Url=https://<your host>/repo
RuntimeRepo=https://dl.flathub.org/repo/flathub.flatpakrepo
IsRuntime=false
GPGKey=<base64 of your public key>
```
