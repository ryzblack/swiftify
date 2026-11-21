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

//! the music services the app reads its catalog and library from: spotify's
//! web api and youtube music. every service answers the same calls with the
//! same shapes (the front end's Song, Album, Artist, Playlist), so the library
//! and the views do not know which one they are showing. a new service is one
//! `MusicService` impl and one arm in `call`.

use std::{collections::HashMap, future::Future};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub mod spotify;
pub mod youtube;

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Artist {
    pub id: String,
    pub name: String,
    #[serde(rename = "artworkURL")]
    pub artwork_url: String,
    pub uri: String,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Album {
    pub id: String,
    pub name: String,
    pub artists: String,
    #[serde(rename = "artworkURL")]
    pub artwork_url: String,
    pub release_date: String,
    pub song_count: i64,
    pub uri: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Song {
    pub name: String,
    pub artists: String,
    pub album_name: String,
    #[serde(rename = "artworkURL")]
    pub artwork_url: String,
    pub uri: String,
    pub duration_ms: i64,
    pub is_explicit: bool,
    #[serde(rename = "albumID")]
    pub album_id: String,
    #[serde(rename = "albumURI")]
    pub album_uri: String,
    pub artist_items: Vec<Artist>,
    pub is_playable: bool,
}

impl Default for Song {
    fn default() -> Self {
        Self {
            name: String::new(),
            artists: String::new(),
            album_name: String::new(),
            artwork_url: String::new(),
            uri: String::new(),
            duration_ms: 0,
            is_explicit: false,
            album_id: String::new(),
            album_uri: String::new(),
            artist_items: Vec::new(),
            is_playable: true,
        }
    }
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub description: String,
    pub song_count: i64,
    #[serde(rename = "artworkURL")]
    pub artwork_url: String,
    pub owner_id: String,
    pub owner_name: String,
    pub is_public: bool,
    pub is_collaborative: bool,
    pub uri: String,
}

#[derive(Serialize, Default, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub display_name: String,
    #[serde(rename = "artworkURL")]
    pub artwork_url: String,
}

#[derive(Serialize, Default, Clone, Debug)]
pub struct SearchResults {
    pub songs: Vec<Song>,
    pub albums: Vec<Album>,
    pub artists: Vec<Artist>,
    pub playlists: Vec<Playlist>,
}

#[derive(Serialize, Default)]
pub struct ArtistGenres {
    pub genres: HashMap<String, Vec<String>>,
    pub artwork: HashMap<String, String>,
}

/// the front end's SpotifyError::Kind, by name
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    InvalidUrl,
    InvalidData,
    Unauthorized,
    Forbidden,
    RateLimited,
    RequestFailed,
    LoginRevoked,
    Unavailable,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ServiceError {
    pub kind: ErrorKind,
    pub message: String,
    /// seconds, or -1 when the service did not say
    pub retry_after: i32,
}

impl ServiceError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            retry_after: -1,
        }
    }
}

pub type Result<T> = std::result::Result<T, ServiceError>;

/// one music service. the access token is the front end's for services it
/// signs in to itself (spotify's oauth); services that keep their own login
/// ignore it.
pub trait MusicService: Sync {
    fn profile(&self, token: &str) -> impl Future<Output = Result<Profile>> + Send;
    fn playlists(&self, token: &str) -> impl Future<Output = Result<Vec<Playlist>>> + Send;
    fn playlist_songs(
        &self,
        id: &str,
        token: &str,
    ) -> impl Future<Output = Result<Vec<Song>>> + Send;
    fn saved_songs(&self, token: &str) -> impl Future<Output = Result<Vec<Song>>> + Send;
    fn saved_song_total(&self, token: &str) -> impl Future<Output = Result<i64>> + Send;
    fn recently_played(&self, token: &str) -> impl Future<Output = Result<Vec<Song>>> + Send;
    fn search(
        &self,
        query: &str,
        token: &str,
    ) -> impl Future<Output = Result<SearchResults>> + Send;
    fn song(&self, id: &str, token: &str) -> impl Future<Output = Result<Song>> + Send;
    fn album(&self, id: &str, token: &str) -> impl Future<Output = Result<Album>> + Send;
    fn artist(&self, id: &str, token: &str) -> impl Future<Output = Result<Artist>> + Send;
    fn playlist(&self, id: &str, token: &str) -> impl Future<Output = Result<Playlist>> + Send;
    fn album_songs(
        &self,
        album: &Album,
        token: &str,
    ) -> impl Future<Output = Result<Vec<Song>>> + Send;
    fn artist_albums(
        &self,
        artist: &Artist,
        token: &str,
    ) -> impl Future<Output = Result<Vec<Album>>> + Send;
    fn artist_genres(
        &self,
        ids: &[String],
        token: &str,
    ) -> impl Future<Output = Result<ArtistGenres>> + Send;
    fn create_playlist(
        &self,
        name: &str,
        description: &str,
        is_public: bool,
        token: &str,
    ) -> impl Future<Output = Result<Playlist>> + Send;
    fn update_playlist(
        &self,
        id: &str,
        name: &str,
        description: &str,
        is_public: bool,
        token: &str,
    ) -> impl Future<Output = Result<()>> + Send;
    fn add_to_playlist(
        &self,
        uri: &str,
        playlist_id: &str,
        token: &str,
    ) -> impl Future<Output = Result<()>> + Send;
    fn remove_from_playlist(
        &self,
        uri: &str,
        playlist_id: &str,
        token: &str,
    ) -> impl Future<Output = Result<()>> + Send;
    fn reorder_playlist(
        &self,
        playlist_id: &str,
        range_start: i64,
        insert_before: i64,
        length: i64,
        token: &str,
    ) -> impl Future<Output = Result<()>> + Send;
    fn set_playlist_image(
        &self,
        playlist_id: &str,
        jpeg_base64: &str,
        token: &str,
    ) -> impl Future<Output = Result<()>> + Send;
    fn delete_playlist(&self, id: &str, token: &str) -> impl Future<Output = Result<()>> + Send;
    fn set_saved(
        &self,
        saved: bool,
        uri: &str,
        token: &str,
    ) -> impl Future<Output = Result<()>> + Send;
    fn saved_statuses(
        &self,
        uris: &[String],
        token: &str,
    ) -> impl Future<Output = Result<HashMap<String, bool>>> + Send;

    /// calls only this service has (its own sign-in, for one)
    fn extension(&self, method: &str, _args: &Value) -> impl Future<Output = Result<Value>> + Send {
        let message = format!("this service has no call named {method}");
        async move { Err(ServiceError::new(ErrorKind::InvalidData, message)) }
    }

    /// what the service keeps between runs (its login), when it changed
    /// since it was last taken: the front end keeps it in the keychain
    fn take_changed_state(&self) -> Option<String> {
        None
    }
}

fn arg<'a>(args: &'a Value, key: &str) -> &'a str {
    args.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn arg_bool(args: &Value, key: &str) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or_default()
}

fn arg_int(args: &Value, key: &str) -> i64 {
    args.get(key).and_then(Value::as_i64).unwrap_or_default()
}

fn arg_strings(args: &Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn arg_object<T: serde::de::DeserializeOwned + Default>(args: &Value, key: &str) -> T {
    args.get(key)
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or_default()
}

fn to_value<T: Serialize>(value: T) -> Result<Value> {
    serde_json::to_value(value)
        .map_err(|error| ServiceError::new(ErrorKind::InvalidData, error.to_string()))
}

async fn call_on<S: MusicService>(
    service: &S,
    method: &str,
    args: &Value,
    token: &str,
) -> Result<Value> {
    match method {
        "profile" => to_value(service.profile(token).await?),
        "playlists" => to_value(service.playlists(token).await?),
        "playlistSongs" => to_value(service.playlist_songs(arg(args, "id"), token).await?),
        "savedSongs" => to_value(service.saved_songs(token).await?),
        "savedSongTotal" => to_value(service.saved_song_total(token).await?),
        "recentlyPlayed" => to_value(service.recently_played(token).await?),
        "search" => to_value(service.search(arg(args, "query"), token).await?),
        "song" => to_value(service.song(arg(args, "id"), token).await?),
        "album" => to_value(service.album(arg(args, "id"), token).await?),
        "artist" => to_value(service.artist(arg(args, "id"), token).await?),
        "playlist" => to_value(service.playlist(arg(args, "id"), token).await?),
        "albumSongs" => to_value(
            service
                .album_songs(&arg_object(args, "album"), token)
                .await?,
        ),
        "artistAlbums" => to_value(
            service
                .artist_albums(&arg_object(args, "artist"), token)
                .await?,
        ),
        "artistGenres" => to_value(
            service
                .artist_genres(&arg_strings(args, "ids"), token)
                .await?,
        ),
        "createPlaylist" => to_value(
            service
                .create_playlist(
                    arg(args, "name"),
                    arg(args, "description"),
                    arg_bool(args, "isPublic"),
                    token,
                )
                .await?,
        ),
        "updatePlaylist" => to_value(
            service
                .update_playlist(
                    arg(args, "id"),
                    arg(args, "name"),
                    arg(args, "description"),
                    arg_bool(args, "isPublic"),
                    token,
                )
                .await?,
        ),
        "addToPlaylist" => to_value(
            service
                .add_to_playlist(arg(args, "uri"), arg(args, "playlistId"), token)
                .await?,
        ),
        "removeFromPlaylist" => to_value(
            service
                .remove_from_playlist(arg(args, "uri"), arg(args, "playlistId"), token)
                .await?,
        ),
        "reorderPlaylist" => to_value(
            service
                .reorder_playlist(
                    arg(args, "playlistId"),
                    arg_int(args, "rangeStart"),
                    arg_int(args, "insertBefore"),
                    arg_int(args, "length"),
                    token,
                )
                .await?,
        ),
        "setPlaylistImage" => to_value(
            service
                .set_playlist_image(arg(args, "playlistId"), arg(args, "jpeg"), token)
                .await?,
        ),
        "deletePlaylist" => to_value(service.delete_playlist(arg(args, "id"), token).await?),
        "setSaved" => to_value(
            service
                .set_saved(arg_bool(args, "saved"), arg(args, "uri"), token)
                .await?,
        ),
        "savedStatuses" => to_value(
            service
                .saved_statuses(&arg_strings(args, "uris"), token)
                .await?,
        ),
        other => service.extension(other, args).await,
    }
}

/// answers one call on the named service. completes with
/// `{"value": <result>, "state": <changed login, when it changed>}`, or fails
/// with a ServiceError as json.
pub async fn call(
    service: &str,
    method: &str,
    args: &str,
    token: &str,
) -> std::result::Result<String, String> {
    let fail = |error: ServiceError| {
        serde_json::to_string(&error).unwrap_or_else(|_| error.message.clone())
    };
    let args: Value = if args.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(args)
            .map_err(|error| fail(ServiceError::new(ErrorKind::InvalidData, error.to_string())))?
    };
    let (result, state) = match service {
        "spotify" => {
            let service = spotify::service();
            (
                call_on(service, method, &args, token).await,
                service.take_changed_state(),
            )
        }
        "youtube" => {
            let service = youtube::service();
            (
                call_on(service, method, &args, token).await,
                service.take_changed_state(),
            )
        }
        other => (
            Err(ServiceError::new(
                ErrorKind::InvalidData,
                format!("there is no music service named {other}"),
            )),
            None,
        ),
    };
    let mut envelope = json!({ "value": result.map_err(fail)? });
    if let Some(state) = state {
        envelope["state"] = Value::String(state);
    }
    Ok(envelope.to_string())
}

/// which service a uri belongs to: `spotify:…`, `ytmusic:…`
pub fn service_of(uri: &str) -> &'static str {
    if uri.starts_with(youtube::URI_PREFIX) {
        "youtube"
    } else {
        "spotify"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn song_json_matches_the_front_end_keys() {
        let song = Song {
            name: "a".into(),
            uri: "spotify:track:x".into(),
            album_id: "al".into(),
            artwork_url: "https://i".into(),
            ..Song::default()
        };
        let value = serde_json::to_value(&song).unwrap();
        assert_eq!(value["albumID"], "al");
        assert_eq!(value["artworkURL"], "https://i");
        assert_eq!(value["isPlayable"], true);
        let back: Song = serde_json::from_value(json!({"name": "b"})).unwrap();
        assert!(
            back.is_playable,
            "a song without the key is playable, as Song::fromJson reads it"
        );
    }

    #[test]
    fn errors_name_the_front_end_kind() {
        let text =
            serde_json::to_string(&ServiceError::new(ErrorKind::RateLimited, "slow")).unwrap();
        assert_eq!(
            text,
            r#"{"kind":"RateLimited","message":"slow","retryAfter":-1}"#
        );
    }

    #[tokio::test]
    async fn calls_answer_in_an_envelope_and_fail_as_service_errors() {
        let status = call("youtube", "status", "", "").await.unwrap();
        let status: Value = serde_json::from_str(&status).unwrap();
        assert_eq!(status["value"]["signedIn"], false);

        let unknown: Value =
            serde_json::from_str(&call("tidal", "search", "{}", "").await.unwrap_err()).unwrap();
        assert_eq!(unknown["kind"], "InvalidData");
        let method: Value =
            serde_json::from_str(&call("spotify", "fly", "{}", "").await.unwrap_err()).unwrap();
        assert!(method["message"].as_str().unwrap().contains("fly"));
        let garbled: Value = serde_json::from_str(
            &call("spotify", "search", "{not json", "")
                .await
                .unwrap_err(),
        )
        .unwrap();
        assert_eq!(garbled["kind"], "InvalidData");
    }

    #[test]
    fn uris_name_their_service() {
        assert_eq!(service_of("ytmusic:track:abc"), "youtube");
        assert_eq!(service_of("spotify:track:abc"), "spotify");
    }
}
