/*
 * Swiftify, a spotify player for desktop
 * Copyright (C) 2026  shred0s
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 */

//! youtube music, through its own web client's api (innertube, read by
//! rustypipe). the catalog works signed out; the library, likes, playlists
//! and history need the account's cookies from music.youtube.com.
//!
//! uris: `ytmusic:track:<video id>`, `ytmusic:album:<browse id>`,
//! `ytmusic:artist:<channel id>`, `ytmusic:playlist:<playlist id>`.
//!
//! what rustypipe keeps between runs (the cookies among it) never touches the
//! disk here: it is handed to the front end, which keeps it in the keychain.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rustypipe::{
    cache::CacheStorage,
    client::{ClientType, RustyPipe, RustyPipeQuery},
    error::{AuthError, Error as PipeError, ExtractionError},
    model::{
        AlbumItem, ArtistId, ArtistItem, MusicAlbum, MusicPlaylist, MusicPlaylistItem, Thumbnail,
        TrackItem, richtext::ToPlaintext,
    },
};
use serde_json::{Value, json};
use sha1::{Digest, Sha1};

use super::{
    Album, Artist, ArtistGenres, ErrorKind, MusicService, Playlist, Profile, Result, SearchResults,
    ServiceError, Song,
};

pub const URI_PREFIX: &str = "ytmusic:";
const ORIGIN: &str = "https://music.youtube.com";
/// the most songs read from one list: a liked songs list can run to thousands
const LIST_LIMIT: usize = 5_000;
const SEARCH_LIMIT: usize = 10;

/// rustypipe's cache, kept in memory and handed out when it changes
#[derive(Default)]
struct State {
    data: Mutex<Option<String>>,
    changed: AtomicBool,
}

struct StateStorage(Arc<State>);

impl CacheStorage for StateStorage {
    fn write(&self, data: &str) {
        if let Ok(mut stored) = self.0.data.lock() {
            *stored = Some(data.to_owned());
            self.0.changed.store(true, Ordering::Release);
        }
    }

    fn read(&self) -> Option<String> {
        self.0.data.lock().ok()?.clone()
    }
}

pub struct YouTube {
    state: Arc<State>,
    /// the signed-in account's visitor data, for the player
    account_visitor: Mutex<Option<String>>,
    /// the login last handed to the front end
    reported: Mutex<Option<String>>,
    client: RwLock<Option<RustyPipe>>,
    http: reqwest::Client,
}

pub fn service() -> &'static YouTube {
    static SERVICE: OnceLock<YouTube> = OnceLock::new();
    SERVICE.get_or_init(|| YouTube {
        state: Arc::default(),
        account_visitor: Mutex::new(None),
        reported: Mutex::new(None),
        client: RwLock::new(None),
        http: reqwest::Client::builder()
            .read_timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default(),
    })
}

// ---------------------------------------------------------------------------
// uris and ids
// ---------------------------------------------------------------------------

fn uri(kind: &str, id: &str) -> String {
    format!("{URI_PREFIX}{kind}:{id}")
}

/// the id at the end of a uri, or the argument itself when it is a bare id
fn id_of(value: &str) -> &str {
    value.rsplit(':').next().unwrap_or(value)
}

/// a video id as youtube makes them; anything else is refused, as it ends up
/// in a file name
pub fn valid_video_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

// ---------------------------------------------------------------------------
// youtube's items as the front end's
// ---------------------------------------------------------------------------

/// the largest cover, which youtube serves at any size asked for
fn artwork(thumbnails: &[Thumbnail]) -> String {
    thumbnails
        .iter()
        .max_by_key(|thumbnail| thumbnail.width)
        .map(|thumbnail| thumbnail.url.clone())
        .unwrap_or_default()
}

fn artist_items(artists: &[ArtistId]) -> Vec<Artist> {
    artists
        .iter()
        .filter_map(|artist| {
            let id = artist.id.as_ref()?;
            Some(Artist {
                id: id.clone(),
                name: artist.name.clone(),
                artwork_url: String::new(),
                uri: uri("artist", id),
            })
        })
        .collect()
}

fn joined(artists: &[ArtistId]) -> String {
    artists
        .iter()
        .map(|artist| artist.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn song(track: &TrackItem, album: Option<&Album>) -> Song {
    let (album_id, album_name) = match (&track.album, album) {
        (Some(id), _) => (id.id.clone(), id.name.clone()),
        (None, Some(album)) => (album.id.clone(), album.name.clone()),
        (None, None) => (String::new(), String::new()),
    };
    let mut artwork_url = artwork(&track.cover);
    if artwork_url.is_empty()
        && let Some(album) = album
    {
        artwork_url = album.artwork_url.clone();
    }
    Song {
        name: track.name.clone(),
        artists: joined(&track.artists),
        album_name,
        artwork_url,
        uri: uri("track", &track.id),
        duration_ms: i64::from(track.duration.unwrap_or_default()) * 1_000,
        is_explicit: false,
        album_uri: if album_id.is_empty() {
            String::new()
        } else {
            uri("album", &album_id)
        },
        album_id,
        artist_items: artist_items(&track.artists),
        is_playable: true,
    }
}

pub fn album_item(item: &AlbumItem) -> Album {
    Album {
        id: item.id.clone(),
        name: item.name.clone(),
        artists: joined(&item.artists),
        artwork_url: artwork(&item.cover),
        release_date: item.year.map(|year| year.to_string()).unwrap_or_default(),
        song_count: 0,
        uri: uri("album", &item.id),
    }
}

pub fn album(album: &MusicAlbum) -> Album {
    Album {
        id: album.id.clone(),
        name: album.name.clone(),
        artists: joined(&album.artists),
        artwork_url: artwork(&album.cover),
        release_date: album.year.map(|year| year.to_string()).unwrap_or_default(),
        song_count: i64::from(album.track_count),
        uri: uri("album", &album.id),
    }
}

pub fn artist_item(item: &ArtistItem) -> Artist {
    Artist {
        id: item.id.clone(),
        name: item.name.clone(),
        artwork_url: artwork(&item.avatar),
        uri: uri("artist", &item.id),
    }
}

pub fn playlist_item(item: &MusicPlaylistItem) -> Playlist {
    Playlist {
        id: item.id.clone(),
        name: item.name.clone(),
        description: String::new(),
        song_count: item.track_count.unwrap_or_default() as i64,
        artwork_url: artwork(&item.thumbnail),
        owner_id: item
            .channel
            .as_ref()
            .map(|channel| channel.id.clone())
            .unwrap_or_default(),
        owner_name: item
            .channel
            .as_ref()
            .map(|channel| channel.name.clone())
            .unwrap_or_else(|| "YouTube Music".into()),
        is_public: false,
        is_collaborative: false,
        uri: uri("playlist", &item.id),
    }
}

pub fn playlist(list: &MusicPlaylist) -> Playlist {
    Playlist {
        id: list.id.clone(),
        name: list.name.clone(),
        description: list
            .description
            .as_ref()
            .map(|text| text.to_plaintext())
            .unwrap_or_default(),
        song_count: list.track_count.unwrap_or(list.tracks.items.len() as u64) as i64,
        artwork_url: artwork(&list.thumbnail),
        owner_id: list
            .channel
            .as_ref()
            .map(|channel| channel.id.clone())
            .unwrap_or_default(),
        owner_name: list
            .channel
            .as_ref()
            .map(|channel| channel.name.clone())
            .unwrap_or_else(|| "YouTube Music".into()),
        is_public: false,
        is_collaborative: false,
        uri: uri("playlist", &list.id),
    }
}

fn error(error: PipeError) -> ServiceError {
    match error {
        PipeError::Auth(AuthError::NoLogin) => ServiceError::new(
            ErrorKind::LoginRevoked,
            "Sign in to YouTube Music in the settings to use your library.",
        ),
        PipeError::Auth(other) => ServiceError::new(
            ErrorKind::LoginRevoked,
            format!("YouTube Music sign-in failed: {other}"),
        ),
        PipeError::Extraction(
            ExtractionError::NotFound { .. } | ExtractionError::Unavailable { .. },
        ) => ServiceError::new(ErrorKind::Unavailable, "YouTube Music will not play this."),
        PipeError::HttpStatus(429, _) => ServiceError::new(
            ErrorKind::RateLimited,
            "YouTube Music is limiting requests. Try again shortly.",
        ),
        PipeError::HttpStatus(401 | 403, _) => ServiceError::new(
            ErrorKind::Forbidden,
            "YouTube Music refused this. Sign in again if it keeps happening.",
        ),
        other => ServiceError::new(
            ErrorKind::RequestFailed,
            format!("YouTube Music request failed: {other}"),
        ),
    }
}

fn http_error(error: reqwest::Error) -> ServiceError {
    ServiceError::new(
        ErrorKind::RequestFailed,
        format!("YouTube Music request failed: {error}"),
    )
}

// ---------------------------------------------------------------------------
// the client
// ---------------------------------------------------------------------------

impl YouTube {
    fn client(&self) -> Result<RustyPipe> {
        if let Some(client) = self.client.read().ok().and_then(|client| client.clone()) {
            return Ok(client);
        }
        let mut slot = self.client.write().map_err(|_| {
            ServiceError::new(
                ErrorKind::RequestFailed,
                "the YouTube Music client is unusable",
            )
        })?;
        if let Some(client) = slot.as_ref() {
            return Ok(client.clone());
        }
        // no reports: rustypipe would write failed responses to disk for its
        // own debugging, and they hold what the person searched and played
        let client = RustyPipe::builder()
            .storage(Box::new(StateStorage(Arc::clone(&self.state))))
            .no_reporter()
            .build()
            .map_err(error)?;
        *slot = Some(client.clone());
        Ok(client)
    }

    fn query(&self) -> Result<RustyPipeQuery> {
        Ok(self.client()?.query())
    }

    fn signed_in(&self) -> bool {
        self.client()
            .map(|client| client.query().auth_enabled(ClientType::DesktopMusic))
            .unwrap_or(false)
    }

    /// what the front end kept from an earlier run. the client is built again
    /// from it, as rustypipe reads its cache only when it is built
    fn restore_state(&self, state: &str) {
        self.forget_account_visitor();
        if let Ok(mut reported) = self.reported.lock() {
            *reported = Some(state.to_owned());
        }
        if let Ok(mut data) = self.state.data.lock() {
            *data = (!state.is_empty()).then(|| state.to_owned());
        }
        if let Ok(mut client) = self.client.write() {
            *client = None;
        }
    }

    /// the cookie header the account signed in with
    fn cookie(&self) -> Option<String> {
        let data = self.state.data.lock().ok()?.clone()?;
        let value: Value = serde_json::from_str(&data).ok()?;
        value
            .pointer("/auth_cookie/cookie")
            .and_then(Value::as_str)
            .map(str::to_owned)
    }

    async fn innertube(&self, endpoint: &str, body: Value) -> Result<Value> {
        let response = self
            .query()?
            .authenticated()
            .raw(ClientType::DesktopMusic, endpoint, &body)
            .await
            .map_err(error)?;
        serde_json::from_str(&response).map_err(|failure| {
            ServiceError::new(
                ErrorKind::InvalidData,
                format!("YouTube Music sent something unreadable: {failure}"),
            )
        })
    }

    async fn edit_playlist(&self, playlist_id: &str, actions: Value) -> Result<()> {
        let response = self
            .innertube(
                "browse/edit_playlist",
                json!({"playlistId": id_of(playlist_id), "actions": actions}),
            )
            .await?;
        match response.get("status").and_then(Value::as_str) {
            Some("STATUS_SUCCEEDED") | None => Ok(()),
            Some(status) => Err(ServiceError::new(
                ErrorKind::Forbidden,
                format!("YouTube Music did not change the playlist: {status}"),
            )),
        }
    }

    /// each entry's id within a playlist (setVideoId), in order: moving songs
    /// names them by it
    async fn playlist_entries(&self, playlist_id: &str) -> Result<Vec<String>> {
        let mut entries = Vec::new();
        let mut page = self
            .innertube(
                "browse",
                json!({"browseId": format!("VL{}", id_of(playlist_id))}),
            )
            .await?;
        for _ in 0..(LIST_LIMIT / 100) {
            collect_set_video_ids(&page, &mut entries);
            let Some(token) = continuation(&page) else {
                break;
            };
            page = self
                .innertube("browse", json!({"continuation": token}))
                .await?;
        }
        Ok(entries)
    }

    async fn liked_track_ids(&self) -> Result<HashSet<String>> {
        let query = self.query()?;
        let mut liked = query.music_liked_tracks().await.map_err(error)?;
        liked
            .tracks
            .extend_limit(&query, LIST_LIMIT)
            .await
            .map_err(error)?;
        Ok(liked
            .tracks
            .items
            .into_iter()
            .map(|track| track.id)
            .collect())
    }

    /// adds the song to the account's youtube history, as youtube's own
    /// player does when a song starts: the history then follows the account
    /// to every device. nothing is sent signed out.
    async fn report_playback(&self, video_id: &str) -> Result<Value> {
        let Some(cookie) = self.cookie() else {
            return Ok(json!({"reported": false}));
        };
        if !valid_video_id(video_id) {
            return Err(ServiceError::new(
                ErrorKind::InvalidData,
                "not a YouTube video id",
            ));
        }
        let player = self
            .innertube("player", json!({"videoId": video_id}))
            .await?;
        let Some(base) = player
            .pointer("/playbackTracking/videostatsPlaybackUrl/baseUrl")
            .and_then(Value::as_str)
        else {
            return Ok(json!({"reported": false}));
        };
        let mut request = self
            .http
            .get(base)
            .query(&[
                ("ver", "2"),
                ("c", "WEB_REMIX"),
                ("cpn", &client_playback_nonce()),
            ])
            .header(reqwest::header::COOKIE, &cookie)
            .header(reqwest::header::ORIGIN, ORIGIN)
            .header("X-Origin", ORIGIN);
        if let Some(authorization) = sapisid_hash(&cookie, unix_seconds()) {
            request = request.header(reqwest::header::AUTHORIZATION, authorization);
        }
        request
            .send()
            .await
            .map_err(http_error)?
            .error_for_status()
            .map_err(http_error)?;
        Ok(json!({"reported": true}))
    }

    async fn lyrics_json(&self, video_id: &str) -> Result<Value> {
        let query = self.query()?;
        let details = query.music_details(video_id).await.map_err(error)?;
        let Some(lyrics_id) = details.lyrics_id else {
            return Err(ServiceError::new(
                ErrorKind::Unavailable,
                "YouTube Music has no lyrics for this song.",
            ));
        };
        let lyrics = query.music_lyrics(&lyrics_id).await.map_err(error)?;
        Ok(lyrics_payload(&lyrics.body, &lyrics.footer))
    }

    /// the song's audio as a file in `dir`, downloaded once: the core plays
    /// it as it plays a local file, through the equalizer and the visualizer.
    /// the folder is kept under `limit` bytes, the least recently played
    /// going first.
    pub async fn audio_file(
        &self,
        video_id: &str,
        dir: &Path,
        limit: u64,
    ) -> std::result::Result<PathBuf, String> {
        if !valid_video_id(video_id) {
            return Err("not a YouTube video id".to_owned());
        }
        let path = dir.join(format!("{video_id}.m4a"));
        if path.is_file() {
            // touched, so pruning keeps it as recently played
            if let Ok(file) = std::fs::File::options().append(true).open(&path) {
                let _ = file.set_modified(SystemTime::now());
            }
            return Ok(path);
        }
        std::fs::create_dir_all(dir)
            .map_err(|failure| format!("could not make the YouTube audio folder: {failure}"))?;
        let partial = dir.join(format!("{video_id}.download.m4a"));
        if let Err(failure) = self.fetch_audio(video_id, &partial).await {
            let _ = std::fs::remove_file(&partial);
            return Err(failure);
        }
        std::fs::rename(&partial, &path)
            .map_err(|failure| format!("could not keep the YouTube audio: {failure}"))?;
        prune(dir, limit, &path);
        Ok(path)
    }
}

impl YouTube {
    /// the audio, from innertube (youtube_stream), with the visitor data the
    /// player wants: the account's when signed in, else rustypipe's, and the
    /// song's watch page's once more when youtube turns the first away.
    /// nothing runs outside the app
    async fn fetch_audio(&self, video_id: &str, partial: &Path) -> std::result::Result<(), String> {
        use super::youtube_stream::{self, StreamError, VISIONOS};
        log::info!(target: "ytmusic", "YTMusic: Resolving audio source video={video_id}");
        let cookie = self.cookie();
        let first = match &cookie {
            Some(cookie) => self.account_visitor_data(cookie).await,
            None => match self.query() {
                Ok(query) => query
                    .get_visitor_data(false)
                    .await
                    .map_err(|failure| error(failure).message),
                Err(failure) => Err(failure.message),
            },
        };
        let mut attempt = match first {
            Ok(visitor_data) => {
                youtube_stream::resolve(&self.http, &VISIONOS, video_id, &visitor_data, None).await
            }
            Err(failure) => Err(StreamError::Failed(failure)),
        };
        // signed in and refused: the same request carrying the account's login
        if let (Err(StreamError::Unplayable(reason)), Some(cookie)) = (&attempt, &cookie) {
            log::warn!(target: "ytmusic", "YTMusic: refused anonymously video={video_id}: {reason}; asking signed in");
            let visitor_data = self.account_visitor_data(cookie).await.unwrap_or_default();
            if let Some(authorization) = sapisid_hash(cookie, unix_seconds()) {
                attempt = youtube_stream::resolve(
                    &self.http,
                    &VISIONOS,
                    video_id,
                    &visitor_data,
                    Some((cookie.as_str(), authorization.as_str())),
                )
                .await;
            }
        }
        if let Err(first_failure) = &attempt {
            log::warn!(target: "ytmusic", "YTMusic: first attempt refused video={video_id}: {first_failure:?}");
            let watch = format!("{ORIGIN}/watch?v={video_id}");
            attempt = match youtube_stream::page_visitor_data(&self.http, &watch, cookie.as_deref())
                .await
            {
                Ok(visitor_data) => {
                    youtube_stream::resolve(&self.http, &VISIONOS, video_id, &visitor_data, None)
                        .await
                }
                Err(page_failure) => Err(page_failure),
            };
        }
        let result = match attempt {
            Ok(stream) => youtube_stream::download(&self.http, &VISIONOS, &stream, partial).await,
            Err(StreamError::Unplayable(reason)) => Err(refusal(&reason, cookie.is_some())),
            Err(StreamError::Failed(failure)) => {
                Err(format!("could not reach YouTube Music's player: {failure}"))
            }
        };
        if let Err(failure) = &result {
            log::warn!(target: "ytmusic", "YTMusic: Playback failed video={video_id}: {failure}");
        }
        result
    }

    /// the signed-in account's visitor data, read once from music.youtube.com
    /// with its cookies; signing in or out forgets it
    async fn account_visitor_data(&self, cookie: &str) -> std::result::Result<String, String> {
        if let Some(known) = self
            .account_visitor
            .lock()
            .ok()
            .and_then(|known| known.clone())
        {
            return Ok(known);
        }
        let fresh = super::youtube_stream::page_visitor_data(
            &self.http,
            &format!("{ORIGIN}/"),
            Some(cookie),
        )
        .await
        .map_err(|failure| format!("{failure:?}"))?;
        if let Ok(mut known) = self.account_visitor.lock() {
            *known = Some(fresh.clone());
        }
        Ok(fresh)
    }

    fn forget_account_visitor(&self) {
        if let Ok(mut known) = self.account_visitor.lock() {
            *known = None;
        }
    }
}

/// what youtube's refusal means to the person, in its own words too
fn refusal(reason: &str, signed_in: bool) -> String {
    match (reason.contains("not a bot"), signed_in) {
        (true, false) => format!(
            "YouTube asks to confirm this is not a bot ({reason}). Sign in to YouTube Music under Settings, Account, Music Service, then play the song again."
        ),
        (true, true) => format!(
            "YouTube asks to confirm this is not a bot even signed in ({reason}). Sign in again under Settings, Account, Music Service, or try again later."
        ),
        _ => format!("YouTube Music will not play this song here: {reason}"),
    }
}

/// the login in rustypipe's cache, as a cache rustypipe reads back
fn login_part(cache: &str) -> String {
    let value: Value = serde_json::from_str(cache).unwrap_or_default();
    let mut login = serde_json::Map::new();
    for key in ["auth_cookie", "oauth_token"] {
        if let Some(part) = value.get(key) {
            login.insert(key.to_owned(), part.clone());
        }
    }
    Value::Object(login).to_string()
}

/// every playlistItemData's setVideoId in `value`, in document order
fn collect_set_video_ids(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            if let Some(id) = map
                .get("playlistItemData")
                .and_then(|data| data.get("playlistSetVideoId"))
                .and_then(Value::as_str)
            {
                out.push(id.to_owned());
            }
            for child in map.values() {
                collect_set_video_ids(child, out);
            }
        }
        Value::Array(items) => items
            .iter()
            .for_each(|item| collect_set_video_ids(item, out)),
        _ => {}
    }
}

fn continuation(value: &Value) -> Option<String> {
    match value {
        Value::Object(map) => map
            .get("continuationCommand")
            .and_then(|command| command.get("token"))
            .or_else(|| {
                map.get("nextContinuationData")
                    .and_then(|data| data.get("continuation"))
            })
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| map.values().find_map(continuation)),
        Value::Array(items) => items.iter().find_map(continuation),
        _ => None,
    }
}

/// the moves that put `length` entries from `range_start` before the entry
/// at `insert_before` (spotify's reorder, by index into the list as it was)
fn reorder_actions(
    entries: &[String],
    range_start: usize,
    insert_before: usize,
    length: usize,
) -> Value {
    let successor = entries.get(insert_before);
    let moves = entries.iter().skip(range_start).take(length).map(|moved| {
        let mut action = json!({"action": "ACTION_MOVE_VIDEO_BEFORE", "setVideoId": moved});
        if let Some(successor) = successor {
            action["movedSetVideoIdSuccessor"] = json!(successor);
        }
        action
    });
    Value::Array(moves.collect())
}

fn lyrics_payload(body: &str, footer: &str) -> Value {
    let lines: Vec<Value> = body
        .lines()
        .map(|words| json!({"startTimeMs": 0, "endTimeMs": 0, "words": words}))
        .collect();
    json!({"syncType": "unsynced", "provider": footer.trim(), "language": "", "lines": lines})
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

/// the header youtube's web client signs its own requests with, from the
/// SAPISID cookie
fn sapisid_hash(cookie: &str, now: u64) -> Option<String> {
    let sapisid = cookie.split(';').map(str::trim).find_map(|pair| {
        pair.strip_prefix("SAPISID=")
            .or_else(|| pair.strip_prefix("__Secure-3PAPISID="))
    })?;
    let digest = Sha1::digest(format!("{now} {sapisid} {ORIGIN}"));
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    Some(format!("SAPISIDHASH {now}_{hex}"))
}

/// a fresh id for one playback, as youtube's player makes them
fn client_playback_nonce() -> String {
    use std::hash::{BuildHasher, Hasher};
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_";
    let mut bits = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish();
    (0..16)
        .map(|_| {
            let character = ALPHABET[(bits & 63) as usize] as char;
            bits = bits.rotate_right(6) ^ 0x9e37_79b9_7f4a_7c15;
            character
        })
        .collect()
}

/// keeps `dir`'s finished downloads under `limit` bytes, oldest first,
/// never the one just made
fn prune(dir: &Path, limit: u64, keep: &Path) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(SystemTime, u64, PathBuf)> = read
        .flatten()
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "m4a")
        })
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            Some((
                metadata.modified().unwrap_or(UNIX_EPOCH),
                metadata.len(),
                entry.path(),
            ))
        })
        .collect();
    let mut total: u64 = files.iter().map(|(_, size, _)| size).sum();
    files.sort();
    for (_, size, path) in files {
        if total <= limit {
            break;
        }
        if path != keep && std::fs::remove_file(&path).is_ok() {
            total -= size;
        }
    }
}

// ---------------------------------------------------------------------------
// the calls
// ---------------------------------------------------------------------------

impl MusicService for YouTube {
    async fn profile(&self, _token: &str) -> Result<Profile> {
        if !self.signed_in() {
            return Err(error(PipeError::Auth(AuthError::NoLogin)));
        }
        let menu = self.innertube("account/account_menu", json!({})).await?;
        let header = menu
            .pointer("/actions/0/openPopupAction/popup/multiPageMenuRenderer/header/activeAccountHeaderRenderer")
            .cloned()
            .unwrap_or(Value::Null);
        let name = header
            .pointer("/accountName/runs/0/text")
            .and_then(Value::as_str)
            .unwrap_or("YouTube Music")
            .to_owned();
        let handle = header
            .pointer("/channelHandle/runs/0/text")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let photo = header
            .pointer("/accountPhoto/thumbnails")
            .and_then(Value::as_array)
            .and_then(|photos| photos.last())
            .and_then(|photo| photo.get("url"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        Ok(Profile {
            id: if handle.is_empty() {
                name.clone()
            } else {
                handle.to_owned()
            },
            display_name: name,
            artwork_url: photo.to_owned(),
        })
    }

    async fn playlists(&self, _token: &str) -> Result<Vec<Playlist>> {
        let query = self.query()?;
        let mut saved = query.music_saved_playlists().await.map_err(error)?;
        saved
            .extend_limit(&query, LIST_LIMIT)
            .await
            .map_err(error)?;
        Ok(saved.items.iter().map(playlist_item).collect())
    }

    async fn playlist_songs(&self, id: &str, _token: &str) -> Result<Vec<Song>> {
        let query = self.query()?;
        let mut list = query.music_playlist(id_of(id)).await.map_err(error)?;
        list.tracks
            .extend_limit(&query, LIST_LIMIT)
            .await
            .map_err(error)?;
        Ok(list
            .tracks
            .items
            .iter()
            .map(|track| song(track, None))
            .collect())
    }

    async fn saved_songs(&self, _token: &str) -> Result<Vec<Song>> {
        let query = self.query()?;
        let mut liked = query.music_liked_tracks().await.map_err(error)?;
        liked
            .tracks
            .extend_limit(&query, LIST_LIMIT)
            .await
            .map_err(error)?;
        Ok(liked
            .tracks
            .items
            .iter()
            .map(|track| song(track, None))
            .collect())
    }

    async fn saved_song_total(&self, _token: &str) -> Result<i64> {
        let liked = self.query()?.music_liked_tracks().await.map_err(error)?;
        Ok(liked.track_count.unwrap_or(liked.tracks.items.len() as u64) as i64)
    }

    async fn recently_played(&self, _token: &str) -> Result<Vec<Song>> {
        let history = self.query()?.music_history().await.map_err(error)?;
        Ok(history
            .items
            .iter()
            .take(20)
            .map(|entry| song(&entry.item, None))
            .collect())
    }

    async fn search(&self, query: &str, _token: &str) -> Result<SearchResults> {
        let client = self.query()?;
        let (tracks, albums, artists, playlists) = tokio::join!(
            client.music_search_tracks(query),
            client.music_search_albums(query),
            client.music_search_artists(query),
            client.music_search_playlists(query, false),
        );
        Ok(SearchResults {
            songs: tracks
                .map_err(error)?
                .items
                .items
                .iter()
                .take(SEARCH_LIMIT)
                .map(|track| song(track, None))
                .collect(),
            albums: albums
                .map_err(error)?
                .items
                .items
                .iter()
                .take(SEARCH_LIMIT)
                .map(album_item)
                .collect(),
            artists: artists
                .map_err(error)?
                .items
                .items
                .iter()
                .take(SEARCH_LIMIT)
                .map(artist_item)
                .collect(),
            playlists: playlists
                .map_err(error)?
                .items
                .items
                .iter()
                .take(SEARCH_LIMIT)
                .map(playlist_item)
                .collect(),
        })
    }

    async fn song(&self, id: &str, _token: &str) -> Result<Song> {
        let details = self
            .query()?
            .music_details(id_of(id))
            .await
            .map_err(error)?;
        Ok(song(&details.track, None))
    }

    async fn album(&self, id: &str, _token: &str) -> Result<Album> {
        Ok(album(
            &self.query()?.music_album(id_of(id)).await.map_err(error)?,
        ))
    }

    async fn artist(&self, id: &str, _token: &str) -> Result<Artist> {
        let found = self
            .query()?
            .music_artist(id_of(id), false)
            .await
            .map_err(error)?;
        Ok(Artist {
            artwork_url: artwork(&found.header_image),
            uri: uri("artist", &found.id),
            id: found.id,
            name: found.name,
        })
    }

    async fn playlist(&self, id: &str, _token: &str) -> Result<Playlist> {
        Ok(playlist(
            &self
                .query()?
                .music_playlist(id_of(id))
                .await
                .map_err(error)?,
        ))
    }

    async fn album_songs(&self, wanted: &Album, _token: &str) -> Result<Vec<Song>> {
        let found = self
            .query()?
            .music_album(id_of(&wanted.id))
            .await
            .map_err(error)?;
        let parsed = album(&found);
        Ok(found
            .tracks
            .iter()
            .map(|track| song(track, Some(&parsed)))
            .collect())
    }

    async fn artist_albums(&self, artist: &Artist, _token: &str) -> Result<Vec<Album>> {
        let albums = self
            .query()?
            .music_artist_albums(id_of(&artist.id), None, None)
            .await
            .map_err(error)?;
        Ok(albums.iter().map(album_item).collect())
    }

    /// youtube music files no artist under genres; the artwork is all there is
    async fn artist_genres(&self, ids: &[String], _token: &str) -> Result<ArtistGenres> {
        let query = self.query()?;
        let mut result = ArtistGenres::default();
        for id in ids.iter().take(50) {
            if let Ok(found) = query.music_artist(id_of(id), false).await {
                result.genres.insert(id.clone(), Vec::new());
                let image = artwork(&found.header_image);
                if !image.is_empty() {
                    result.artwork.insert(id.clone(), image);
                }
            }
        }
        Ok(result)
    }

    async fn create_playlist(
        &self,
        name: &str,
        description: &str,
        is_public: bool,
        _token: &str,
    ) -> Result<Playlist> {
        let privacy = if is_public { "PUBLIC" } else { "PRIVATE" };
        let response = self
            .innertube(
                "playlist/create",
                json!({"title": name, "description": description, "privacyStatus": privacy}),
            )
            .await?;
        let id = response
            .get("playlistId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if id.is_empty() {
            return Err(ServiceError::new(
                ErrorKind::InvalidData,
                "the new playlist had no id.",
            ));
        }
        let owner = self.profile("").await.unwrap_or_default();
        Ok(Playlist {
            id: id.to_owned(),
            name: name.to_owned(),
            description: description.to_owned(),
            owner_id: owner.id,
            owner_name: owner.display_name,
            is_public,
            uri: uri("playlist", id),
            ..Playlist::default()
        })
    }

    async fn update_playlist(
        &self,
        id: &str,
        name: &str,
        description: &str,
        is_public: bool,
        _token: &str,
    ) -> Result<()> {
        let privacy = if is_public { "PUBLIC" } else { "PRIVATE" };
        self.edit_playlist(
            id,
            json!([
                {"action": "ACTION_SET_PLAYLIST_NAME", "playlistName": name},
                {"action": "ACTION_SET_PLAYLIST_DESCRIPTION", "playlistDescription": description},
                {"action": "ACTION_SET_PLAYLIST_PRIVACY", "playlistPrivacy": privacy},
            ]),
        )
        .await
    }

    async fn add_to_playlist(&self, song_uri: &str, playlist_id: &str, _token: &str) -> Result<()> {
        self.edit_playlist(
            playlist_id,
            json!([{"action": "ACTION_ADD_VIDEO", "addedVideoId": id_of(song_uri)}]),
        )
        .await
    }

    async fn remove_from_playlist(
        &self,
        song_uri: &str,
        playlist_id: &str,
        _token: &str,
    ) -> Result<()> {
        self.edit_playlist(
            playlist_id,
            json!([{"action": "ACTION_REMOVE_VIDEO_BY_VIDEO_ID", "removedVideoId": id_of(song_uri)}]),
        )
        .await
    }

    async fn reorder_playlist(
        &self,
        playlist_id: &str,
        range_start: i64,
        insert_before: i64,
        length: i64,
        _token: &str,
    ) -> Result<()> {
        let entries = self.playlist_entries(playlist_id).await?;
        let actions = reorder_actions(
            &entries,
            range_start.max(0) as usize,
            insert_before.max(0) as usize,
            length.max(0) as usize,
        );
        if actions.as_array().is_none_or(Vec::is_empty) {
            return Err(ServiceError::new(
                ErrorKind::InvalidData,
                "those songs are not in the playlist.",
            ));
        }
        self.edit_playlist(playlist_id, actions).await
    }

    async fn set_playlist_image(
        &self,
        _playlist_id: &str,
        _jpeg_base64: &str,
        _token: &str,
    ) -> Result<()> {
        Err(ServiceError::new(
            ErrorKind::Forbidden,
            "YouTube Music makes a playlist's cover from its songs.",
        ))
    }

    async fn delete_playlist(&self, id: &str, _token: &str) -> Result<()> {
        self.innertube("playlist/delete", json!({"playlistId": id_of(id)}))
            .await?;
        Ok(())
    }

    async fn set_saved(&self, saved: bool, item_uri: &str, _token: &str) -> Result<()> {
        let id = id_of(item_uri);
        let like = if saved {
            "like/like"
        } else {
            "like/removelike"
        };
        if item_uri.starts_with("ytmusic:artist:") {
            let endpoint = if saved {
                "subscription/subscribe"
            } else {
                "subscription/unsubscribe"
            };
            self.innertube(endpoint, json!({"channelIds": [id]}))
                .await?;
        } else if item_uri.starts_with("ytmusic:album:") {
            // an album is saved through the playlist behind it
            let found = self.query()?.music_album(id).await.map_err(error)?;
            let playlist_id = found.playlist_id.ok_or_else(|| {
                ServiceError::new(ErrorKind::InvalidData, "that album cannot be saved")
            })?;
            self.innertube(like, json!({"target": {"playlistId": playlist_id}}))
                .await?;
        } else if item_uri.starts_with("ytmusic:playlist:") {
            self.innertube(like, json!({"target": {"playlistId": id}}))
                .await?;
        } else {
            self.innertube(like, json!({"target": {"videoId": id}}))
                .await?;
        }
        Ok(())
    }

    async fn saved_statuses(&self, uris: &[String], _token: &str) -> Result<HashMap<String, bool>> {
        let mut result = HashMap::new();
        let wanted: Vec<&String> = uris.iter().take(40).collect();
        if wanted.iter().any(|uri| uri.starts_with("ytmusic:track:")) {
            let liked = self.liked_track_ids().await?;
            for uri in wanted
                .iter()
                .filter(|uri| uri.starts_with("ytmusic:track:"))
            {
                result.insert((*uri).clone(), liked.contains(id_of(uri)));
            }
        }
        if wanted
            .iter()
            .any(|uri| uri.starts_with("ytmusic:playlist:"))
        {
            let saved: HashSet<String> = self
                .playlists("")
                .await?
                .into_iter()
                .map(|list| list.id)
                .collect();
            for uri in wanted
                .iter()
                .filter(|uri| uri.starts_with("ytmusic:playlist:"))
            {
                result.insert((*uri).clone(), saved.contains(id_of(uri)));
            }
        }
        if wanted.iter().any(|uri| uri.starts_with("ytmusic:album:")) {
            let query = self.query()?;
            let mut albums = query.music_saved_albums().await.map_err(error)?;
            albums
                .extend_limit(&query, LIST_LIMIT)
                .await
                .map_err(error)?;
            let saved: HashSet<&str> = albums.items.iter().map(|album| album.id.as_str()).collect();
            for uri in wanted
                .iter()
                .filter(|uri| uri.starts_with("ytmusic:album:"))
            {
                result.insert((*uri).clone(), saved.contains(id_of(uri)));
            }
        }
        Ok(result)
    }

    async fn extension(&self, method: &str, args: &Value) -> Result<Value> {
        let text = |key: &str| {
            args.get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        match method {
            "restoreState" => {
                self.restore_state(&text("state"));
                Ok(json!({"signedIn": self.signed_in()}))
            }
            "status" => Ok(json!({"signedIn": self.signed_in()})),
            // asks youtube whether it still takes the saved cookies
            "checkLogin" => match self.client()?.user_auth_check_cookie().await {
                Ok(()) => Ok(json!({"valid": true})),
                Err(PipeError::Auth(_)) => Ok(json!({"valid": false})),
                Err(other) => Err(error(other)),
            },
            // the Cookie header of a signed-in music.youtube.com tab, or a cookies.txt
            "signIn" => {
                let cookie = text("cookie");
                let client = self.client()?;
                let result = if cookie.trim_start().starts_with('#') || cookie.contains('\t') {
                    client.user_auth_set_cookie_txt(&cookie).await
                } else {
                    client.user_auth_set_cookie(cookie.trim()).await
                };
                result.map_err(error)?;
                self.forget_account_visitor();
                Ok(json!({"signedIn": self.signed_in()}))
            }
            "signOut" => {
                self.client()?
                    .user_auth_remove_cookie()
                    .await
                    .map_err(error)?;
                self.forget_account_visitor();
                Ok(json!({"signedIn": false}))
            }
            "reportPlayback" => self.report_playback(id_of(&text("uri"))).await,
            "lyrics" => self.lyrics_json(id_of(&text("uri"))).await,
            "artistTopTracks" => {
                let found = self
                    .query()?
                    .music_artist(id_of(&text("id")), false)
                    .await
                    .map_err(error)?;
                super::to_value(
                    found
                        .tracks
                        .iter()
                        .map(|track| song(track, None))
                        .collect::<Vec<_>>(),
                )
            }
            other => Err(ServiceError::new(
                ErrorKind::InvalidData,
                format!("YouTube Music has no call named {other}"),
            )),
        }
    }

    /// only the login (the account's cookies) goes to the keychain, and only
    /// when it changed: the rest of rustypipe's cache is fetched again
    fn take_changed_state(&self) -> Option<String> {
        if !self.state.changed.swap(false, Ordering::AcqRel) {
            return None;
        }
        let login = login_part(self.state.data.lock().ok()?.as_deref().unwrap_or_default());
        let mut reported = self.reported.lock().ok()?;
        if reported.as_deref() == Some(login.as_str()) {
            return None;
        }
        *reported = Some(login.clone());
        Some(login)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track() -> TrackItem {
        serde_json::from_value(json!({
            "id": "dQw4w9WgXcQ", "name": "Song", "duration": 212,
            "cover": [
                {"url": "https://small", "width": 60, "height": 60},
                {"url": "https://big", "width": 544, "height": 544},
            ],
            "artists": [{"id": "UC1", "name": "One"}, {"id": null, "name": "Two"}],
            "artist_id": "UC1", "album": null, "view_count": null,
            "track_type": "track", "track_nr": null, "by_va": false,
        }))
        .unwrap()
    }

    #[test]
    fn tracks_become_songs_with_youtube_uris() {
        let fallback = Album {
            id: "MPRE1".into(),
            name: "Album".into(),
            ..Album::default()
        };
        let mapped = song(&track(), Some(&fallback));
        assert_eq!(mapped.uri, "ytmusic:track:dQw4w9WgXcQ");
        assert_eq!(mapped.duration_ms, 212_000);
        assert_eq!(mapped.artists, "One, Two");
        assert_eq!(mapped.artist_items.len(), 1);
        assert_eq!(mapped.artist_items[0].uri, "ytmusic:artist:UC1");
        assert_eq!(mapped.artwork_url, "https://big");
        assert_eq!(mapped.album_uri, "ytmusic:album:MPRE1");

        let mut own_album = track();
        own_album.album =
            Some(serde_json::from_value(json!({"id": "MPRE2", "name": "Other"})).unwrap());
        assert_eq!(song(&own_album, Some(&fallback)).album_name, "Other");
    }

    #[test]
    fn video_ids_are_checked_before_naming_files() {
        assert!(valid_video_id("dQw4w9WgXcQ"));
        assert!(!valid_video_id("../../etc/x"));
        assert!(!valid_video_id("dQw4w9WgXc"));
        assert_eq!(id_of("ytmusic:track:dQw4w9WgXcQ"), "dQw4w9WgXcQ");
        assert_eq!(id_of("dQw4w9WgXcQ"), "dQw4w9WgXcQ");
    }

    #[test]
    fn reorder_moves_before_the_original_successor() {
        let entries: Vec<String> = ["a", "b", "c", "d"].map(String::from).to_vec();
        let actions = reorder_actions(&entries, 0, 3, 2);
        assert_eq!(actions[0]["setVideoId"], "a");
        assert_eq!(actions[1]["setVideoId"], "b");
        assert_eq!(actions[0]["movedSetVideoIdSuccessor"], "d");
        // to the end: no successor
        let to_end = reorder_actions(&entries, 1, 4, 1);
        assert!(to_end[0].get("movedSetVideoIdSuccessor").is_none());
    }

    #[test]
    fn set_video_ids_and_continuations_are_found_anywhere() {
        let page = json!({"contents": [
            {"musicResponsiveListItemRenderer": {"playlistItemData": {"videoId": "v1", "playlistSetVideoId": "s1"}}},
            {"x": {"playlistItemData": {"playlistSetVideoId": "s2"}}},
            {"continuationItemRenderer": {"continuationEndpoint": {"continuationCommand": {"token": "next"}}}},
        ]});
        let mut ids = Vec::new();
        collect_set_video_ids(&page, &mut ids);
        assert_eq!(ids, ["s1", "s2"]);
        assert_eq!(continuation(&page).as_deref(), Some("next"));
    }

    #[test]
    fn sapisid_hash_signs_with_the_cookie() {
        let header = sapisid_hash("a=b; SAPISID=secret; c=d", 1_700_000_000).unwrap();
        // sha1("1700000000 secret https://music.youtube.com")
        let expected = Sha1::digest("1700000000 secret https://music.youtube.com");
        let hex: String = expected.iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(header, format!("SAPISIDHASH 1700000000_{hex}"));
        assert!(sapisid_hash("a=b", 1).is_none());
    }

    #[test]
    fn nonces_are_sixteen_url_safe_characters() {
        let nonce = client_playback_nonce();
        assert_eq!(nonce.len(), 16);
        assert!(
            nonce
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        );
    }

    #[test]
    fn refusals_say_what_to_do() {
        assert!(
            refusal("Sign in to confirm you're not a bot", false)
                .contains("Sign in to YouTube Music")
        );
        assert!(refusal("Sign in to confirm you're not a bot", true).contains("even signed in"));
        assert_eq!(
            refusal("Video unavailable", false),
            "YouTube Music will not play this song here: Video unavailable"
        );
    }

    #[test]
    fn unsynced_lyrics_keep_their_lines() {
        let payload = lyrics_payload("one\ntwo", " Source: X ");
        assert_eq!(payload["lines"][1]["words"], "two");
        assert_eq!(payload["syncType"], "unsynced");
        assert_eq!(payload["provider"], "Source: X");
    }

    #[test]
    fn pruning_keeps_the_newest_and_the_kept() {
        let dir = std::env::temp_dir().join(format!("swiftify-prune-{}", client_playback_nonce()));
        std::fs::create_dir_all(&dir).unwrap();
        for (index, name) in ["old.m4a", "mid.m4a", "new.m4a"].iter().enumerate() {
            let path = dir.join(name);
            std::fs::write(&path, vec![0u8; 100]).unwrap();
            let file = std::fs::File::options().append(true).open(&path).unwrap();
            file.set_modified(UNIX_EPOCH + Duration::from_secs(1_000 + index as u64))
                .unwrap();
        }
        prune(&dir, 200, &dir.join("old.m4a"));
        assert!(dir.join("old.m4a").exists(), "the file just made is kept");
        assert!(!dir.join("mid.m4a").exists());
        assert!(dir.join("new.m4a").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn state_is_handed_out_once_per_change() {
        let youtube = YouTube {
            state: Arc::default(),
            account_visitor: Mutex::new(None),
            reported: Mutex::new(None),
            client: RwLock::new(None),
            http: reqwest::Client::new(),
        };
        assert!(youtube.take_changed_state().is_none());
        let storage = StateStorage(Arc::clone(&youtube.state));
        storage.write(r#"{"clients":{"x":1},"auth_cookie":{"cookie":"SAPISID=x"}}"#);
        assert_eq!(
            youtube.take_changed_state().as_deref(),
            Some(r#"{"auth_cookie":{"cookie":"SAPISID=x"}}"#)
        );
        assert!(youtube.take_changed_state().is_none());
        // a cache change that leaves the login alone is not handed out
        storage.write(r#"{"clients":{"x":2},"auth_cookie":{"cookie":"SAPISID=x"}}"#);
        assert!(youtube.take_changed_state().is_none());
        assert_eq!(youtube.cookie().as_deref(), Some("SAPISID=x"));
    }

    /// against youtube itself: `cargo test --features capi --no-default-features -- --ignored live`
    #[test]
    #[ignore = "reaches youtube music over the network"]
    fn live_search_album_and_audio_decode() {
        use rodio::Source;
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let youtube = service();
            let found = youtube.search("Daft Punk Get Lucky", "").await.unwrap();
            assert!(!found.songs.is_empty(), "search found songs");
            assert!(!found.albums.is_empty() && !found.artists.is_empty());
            let first = &found.songs[0];
            println!(
                "song: {} - {} ({} ms) {}",
                first.artists, first.name, first.duration_ms, first.uri
            );
            let album = found.albums[0].clone();
            let songs = youtube.album_songs(&album, "").await.unwrap();
            assert!(!songs.is_empty(), "album has songs");
            assert!(songs.iter().all(|song| !song.album_id.is_empty()));
            let lyrics = youtube
                .extension("lyrics", &json!({"uri": first.uri}))
                .await;
            println!(
                "lyrics: {}",
                lyrics
                    .map(|value| value["lines"].as_array().map_or(0, Vec::len))
                    .unwrap_or(0)
            );

            // the second way in: the song's watch page's own visitor data
            let watch = format!("{ORIGIN}/watch?v={}", id_of(&first.uri));
            let page_visitor =
                crate::music::youtube_stream::page_visitor_data(&youtube.http, &watch, None)
                    .await
                    .unwrap();
            let stream = crate::music::youtube_stream::resolve(
                &youtube.http,
                &crate::music::youtube_stream::VISIONOS,
                id_of(&first.uri),
                &page_visitor,
                None,
            )
            .await
            .unwrap();
            println!("watch-page stream: {} bytes, {}", stream.size, stream.mime);
            assert!(stream.size > 500_000);

            let dir = std::env::temp_dir().join("swiftify-youtube-live");
            let path = youtube
                .audio_file(id_of(&first.uri), &dir, 64 * 1024 * 1024)
                .await
                .unwrap();
            let size = std::fs::metadata(&path).unwrap().len();
            println!("audio: {} bytes at {}", size, path.display());
            assert!(size > 500_000);
            let file = std::fs::File::open(&path).unwrap();
            let mut decoder = rodio::Decoder::builder()
                .with_data(std::io::BufReader::new(file))
                .with_byte_len(size)
                .with_seekable(true)
                .with_hint("m4a")
                .build()
                .unwrap();
            println!(
                "decoded: {} Hz, {} ch, {:?}",
                decoder.sample_rate(),
                decoder.channels(),
                decoder.total_duration()
            );
            let samples = decoder.by_ref().take(48_000 * 2 * 3).count();
            assert!(samples > 48_000, "decoded three seconds");
            decoder.try_seek(Duration::from_secs(60)).unwrap();
            assert!(decoder.next().is_some(), "plays on after a seek");
        });
    }
}
