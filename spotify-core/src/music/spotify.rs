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

//! spotify's web api: the catalog and the account's library. ported from the
//! qt front end's WebApi and SpotifyJson, call for call.

use std::{
    collections::{HashMap, HashSet},
    sync::OnceLock,
    time::Duration,
};

use reqwest::{Method, Url};
use serde_json::{Value, json};

use super::{
    Album, Artist, ArtistGenres, ErrorKind, MusicService, Playlist, Profile, Result, SearchResults,
    ServiceError, Song,
};

pub const MAXIMUM_COVER_BYTES: usize = 256_000;
const PAGE_SIZE: usize = 50;
const PAGES_AT_ONCE: usize = 4;
/// the account's own market, so spotify says which tracks it will play there
const MARKET: (&str, &str) = ("market", "from_token");

pub struct Spotify {
    http: reqwest::Client,
    /// https://api.spotify.com, or a test's own server
    base: Url,
}

pub fn service() -> &'static Spotify {
    static SERVICE: OnceLock<Spotify> = OnceLock::new();
    // SWIFTIFY_SPOTIFY_API points the calls at a test's stand-in (tests/tst_session.cpp)
    SERVICE.get_or_init(|| {
        let base = std::env::var("SWIFTIFY_SPOTIFY_API")
            .ok()
            .and_then(|url| Url::parse(&url).ok());
        Spotify::new(base.unwrap_or_else(|| api_url("/", &[])))
    })
}

fn invalid_data(detail: &str) -> ServiceError {
    ServiceError::new(
        ErrorKind::InvalidData,
        format!("Could not read Spotify's response: {detail}"),
    )
}

fn request_failed(detail: &str) -> ServiceError {
    ServiceError::new(
        ErrorKind::RequestFailed,
        format!("Spotify API request failed: {detail}"),
    )
}

pub fn api_url(path: &str, query: &[(&str, &str)]) -> Url {
    let mut url =
        Url::parse("https://api.spotify.com").unwrap_or_else(|_| unreachable!("a fixed valid url"));
    url.set_path(path);
    if !query.is_empty() {
        url.query_pairs_mut().extend_pairs(query);
    }
    url
}

impl Spotify {
    fn new(base: Url) -> Self {
        Self {
            http: reqwest::Client::builder()
                .read_timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            base,
        }
    }

    fn url(&self, path: &str, query: &[(&str, &str)]) -> Url {
        let mut url = self.base.clone();
        url.set_path(path);
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        url
    }

    async fn request_data(
        &self,
        url: Url,
        method: Method,
        token: &str,
        body: Option<(Vec<u8>, &str)>,
    ) -> Result<Vec<u8>> {
        if url.scheme() != self.base.scheme()
            || url.host_str() != self.base.host_str()
            || url.port() != self.base.port()
        {
            return Err(ServiceError::new(
                ErrorKind::InvalidUrl,
                "Spotify returned an invalid API URL.",
            ));
        }
        let mut request = self.http.request(method, url).bearer_auth(token);
        if let Some((body, content_type)) = body {
            request = request
                .header(reqwest::header::CONTENT_TYPE, content_type)
                .body(body);
        }
        let response = request
            .send()
            .await
            .map_err(|error| request_failed(&error.to_string()))?;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<i32>().ok());
        let data = response
            .bytes()
            .await
            .map_err(|error| request_failed(&error.to_string()))?
            .to_vec();
        if (200..300).contains(&status) {
            return Ok(data);
        }
        Err(status_error(status, &data, retry_after))
    }

    async fn request_json(
        &self,
        url: Url,
        method: Method,
        token: &str,
        body: Option<(Vec<u8>, &str)>,
    ) -> Result<Value> {
        let data = self.request_data(url, method, token, body).await?;
        serde_json::from_slice(&data).map_err(|error| invalid_data(&error.to_string()))
    }

    async fn get(&self, url: Url, token: &str) -> Result<Value> {
        self.request_json(url, Method::GET, token, None).await
    }

    async fn send_json(
        &self,
        url: Url,
        method: Method,
        token: &str,
        body: Value,
    ) -> Result<Vec<u8>> {
        self.request_data(
            url,
            method,
            token,
            Some((body.to_string().into_bytes(), "application/json")),
        )
        .await
    }

    /// follows a list's `next` links to its end
    async fn collect_pages(
        &self,
        first: Url,
        token: &str,
        mut on_page: impl FnMut(&Value),
    ) -> Result<()> {
        let mut visited = HashSet::new();
        let mut next = Some(first);
        while let Some(url) = next {
            if !visited.insert(url.clone()) {
                return Err(invalid_data("Spotify repeated a page URL."));
            }
            let page = self.get(url, token).await?;
            on_page(&page);
            next = page
                .get("next")
                .and_then(Value::as_str)
                .and_then(|next| Url::parse(next).ok());
        }
        Ok(())
    }

    /// a page, waiting out one short rate limit; a long one is the caller's to report
    async fn request_page(&self, url: Url, token: &str) -> Result<Value> {
        match self.get(url.clone(), token).await {
            Err(error) if error.kind == ErrorKind::RateLimited && error.retry_after <= 10 => {
                tokio::time::sleep(Duration::from_secs(error.retry_after.max(1) as u64)).await;
                self.get(url, token).await
            }
            other => other,
        }
    }

    /// a list read by offset, a few pages at a time once its total is known
    async fn collect_offset_pages(
        &self,
        path: &str,
        query: &[(&str, &str)],
        token: &str,
        mut on_page: impl FnMut(&Value),
    ) -> Result<()> {
        let page_size = PAGE_SIZE.to_string();
        let page_url = |offset: usize| {
            let offset = offset.to_string();
            let mut pairs: Vec<(&str, &str)> = query
                .iter()
                .filter(|(key, _)| *key != "limit")
                .copied()
                .collect();
            pairs.push(("limit", &page_size));
            pairs.push(("offset", &offset));
            self.url(path, &pairs)
        };
        let first = self.request_page(page_url(0), token).await?;
        on_page(&first);
        let total = first
            .get("total")
            .and_then(Value::as_u64)
            .unwrap_or_default() as usize;
        let mut start = PAGE_SIZE;
        while start < total {
            let end = total.min(start + PAGE_SIZE * PAGES_AT_ONCE);
            // started together, then every one awaited to its end even when one
            // fails, so none is left running unowned
            let batch = (start..end)
                .step_by(PAGE_SIZE)
                .map(|offset| self.request_page(page_url(offset), token));
            let pages = futures::future::join_all(batch).await;
            for page in pages.iter() {
                if let Err(error) = page {
                    return Err(error.clone());
                }
            }
            for page in pages.into_iter().flatten() {
                on_page(&page);
            }
            start += PAGE_SIZE * PAGES_AT_ONCE;
        }
        Ok(())
    }
}

fn status_error(status: u16, data: &[u8], retry_after: Option<i32>) -> ServiceError {
    let message = serde_json::from_slice::<Value>(data)
        .ok()
        .and_then(|body| {
            body.pointer("/error/message")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .filter(|message| !message.is_empty())
        .unwrap_or_else(|| format!("HTTP {status}"));
    match status {
        401 => ServiceError::new(
            ErrorKind::Unauthorized,
            format!("Spotify API authorization failed: {message}"),
        ),
        // spotify's own text here is usually the bare word "forbidden"
        403 => ServiceError::new(
            ErrorKind::Forbidden,
            format!("Spotify refused this change: {message}"),
        ),
        429 => ServiceError {
            kind: ErrorKind::RateLimited,
            message: match retry_after {
                Some(seconds) => {
                    format!("Spotify rate limit exceeded. Try again in {seconds} seconds.")
                }
                None => "Spotify rate limit exceeded. Try again shortly.".to_owned(),
            },
            retry_after: retry_after.unwrap_or(-1),
        },
        _ => request_failed(&message),
    }
}

// ---------------------------------------------------------------------------
// reading spotify's json (SpotifyJson.cpp)
// ---------------------------------------------------------------------------

fn string(object: &Value, key: &str) -> String {
    object
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn array<'a>(object: &'a Value, key: &str) -> &'a [Value] {
    object
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn joined_names(artists: &[Value]) -> String {
    artists
        .iter()
        .map(|artist| string(artist, "name"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// spotify sends a playlist's description as html, its apostrophes as &#x27;
/// and its links as tags
pub fn plain_text(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(open) = rest.find('<') {
        let Some(close) = rest[open..].find('>') else {
            break;
        };
        text.push_str(&rest[..open]);
        rest = &rest[open + close + 1..];
    }
    text.push_str(rest);

    let mut result = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while let Some(amp) = rest.find('&') {
        result.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        match after
            .find(';')
            .and_then(|semi| decode_entity(&after[..semi]).map(|decoded| (semi, decoded)))
        {
            Some((semi, decoded)) => {
                result.push(decoded);
                rest = &after[semi + 1..];
            }
            None => {
                result.push('&');
                rest = after;
            }
        }
    }
    result.push_str(rest);
    result
}

fn decode_entity(name: &str) -> Option<char> {
    if let Some(number) = name.strip_prefix('#') {
        let (digits, radix) = match number.strip_prefix(['x', 'X']) {
            Some(hex) => (hex, 16),
            None => (number, 10),
        };
        if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
            return None;
        }
        return u32::from_str_radix(digits, radix)
            .ok()
            .filter(|code| *code > 0)
            .and_then(char::from_u32);
    }
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        _ => return None,
    })
}

pub fn preferred_artwork_url(images: &[Value], preferred_width: i64) -> String {
    let mut best = String::new();
    let mut best_distance = i64::MAX;
    for image in images {
        let url = string(image, "url");
        if Url::parse(&url).is_err() {
            continue;
        }
        let width = image
            .get("width")
            .and_then(Value::as_f64)
            .map_or(preferred_width, |width| width as i64);
        let distance = (width - preferred_width).abs();
        if distance < best_distance {
            best_distance = distance;
            best = url;
        }
    }
    best
}

pub fn profile(user: &Value) -> Profile {
    let name = string(user, "display_name");
    Profile {
        id: string(user, "id"),
        display_name: if name.is_empty() {
            "Spotify User".to_owned()
        } else {
            name
        },
        artwork_url: preferred_artwork_url(array(user, "images"), 192),
    }
}

pub fn playlist(object: &Value) -> Playlist {
    let id = string(object, "id");
    let total = |key: &str| object.get(key).and_then(|value| value.get("total"));
    let owner = object.get("owner").cloned().unwrap_or(Value::Null);
    let uri = string(object, "uri");
    Playlist {
        name: string(object, "name"),
        description: plain_text(&string(object, "description")),
        song_count: total("items")
            .or_else(|| total("tracks"))
            .and_then(Value::as_i64)
            .unwrap_or_default(),
        artwork_url: preferred_artwork_url(array(object, "images"), 640),
        owner_id: string(&owner, "id"),
        owner_name: string(&owner, "display_name"),
        is_public: object
            .get("public")
            .and_then(Value::as_bool)
            .unwrap_or_default(),
        is_collaborative: object
            .get("collaborative")
            .and_then(Value::as_bool)
            .unwrap_or_default(),
        uri: if uri.is_empty() {
            format!("spotify:playlist:{id}")
        } else {
            uri
        },
        id,
    }
}

pub fn album(object: &Value) -> Option<Album> {
    let id = string(object, "id");
    let uri = string(object, "uri");
    if id.is_empty() || uri.is_empty() {
        return None;
    }
    Some(Album {
        id,
        name: string(object, "name"),
        artists: joined_names(array(object, "artists")),
        artwork_url: preferred_artwork_url(array(object, "images"), 640),
        release_date: string(object, "release_date"),
        song_count: object
            .get("total_tracks")
            .and_then(Value::as_i64)
            .unwrap_or_default(),
        uri,
    })
}

pub fn artist(object: &Value) -> Option<Artist> {
    let id = string(object, "id");
    let uri = string(object, "uri");
    if id.is_empty() || uri.is_empty() {
        return None;
    }
    Some(Artist {
        id,
        name: string(object, "name"),
        artwork_url: preferred_artwork_url(array(object, "images"), 640),
        uri,
    })
}

pub fn track(object: &Value, fallback_album: &Album) -> Option<Song> {
    let kind = string(object, "type");
    // a market lets spotify send a relinked copy in its place. playlists and
    // the library hold the original, and librespot finds the copy itself
    let linked = object
        .get("linked_from")
        .map(|linked| string(linked, "uri"))
        .unwrap_or_default();
    let uri = if linked.is_empty() {
        string(object, "uri")
    } else {
        linked
    };
    if (!kind.is_empty() && kind != "track") || !uri.starts_with("spotify:track:") {
        return None;
    }
    let album_object = object.get("album").cloned().unwrap_or(Value::Null);
    let resolved = album(&album_object).unwrap_or_else(|| fallback_album.clone());
    let artists = array(object, "artists");
    let mut artwork_url = preferred_artwork_url(array(&album_object, "images"), 640);
    if artwork_url.is_empty() {
        artwork_url = resolved.artwork_url.clone();
    }
    Some(Song {
        name: string(object, "name"),
        artists: joined_names(artists),
        album_name: resolved.name,
        artwork_url,
        uri,
        duration_ms: object
            .get("duration_ms")
            .and_then(Value::as_i64)
            .unwrap_or_default(),
        is_explicit: object
            .get("explicit")
            .and_then(Value::as_bool)
            .unwrap_or_default(),
        album_id: resolved.id,
        album_uri: resolved.uri,
        artist_items: artists.iter().filter_map(artist).collect(),
        // only sent when the request names a market
        is_playable: object
            .get("is_playable")
            .and_then(Value::as_bool)
            .unwrap_or(true),
    })
}

pub fn wrapped_track(entry: &Value) -> Option<Song> {
    match entry.get("item") {
        Some(item) if item.is_object() => track(item, &Album::default()),
        _ => entry
            .get("track")
            .filter(|legacy| legacy.is_object())
            .and_then(|legacy| track(legacy, &Album::default())),
    }
}

fn wrapped_tracks(songs: &mut Vec<Song>, page: &Value) {
    songs.extend(array(page, "items").iter().filter_map(wrapped_track));
}

// ---------------------------------------------------------------------------
// the calls (WebApi.cpp)
// ---------------------------------------------------------------------------

impl MusicService for Spotify {
    async fn profile(&self, token: &str) -> Result<Profile> {
        let result = profile(&self.get(self.url("/v1/me", &[]), token).await?);
        if result.id.is_empty() {
            return Err(invalid_data("the profile had no id."));
        }
        Ok(result)
    }

    async fn playlists(&self, token: &str) -> Result<Vec<Playlist>> {
        let mut result = Vec::new();
        self.collect_pages(
            self.url("/v1/me/playlists", &[("limit", "50")]),
            token,
            |page| {
                result.extend(
                    array(page, "items")
                        .iter()
                        .filter(|value| value.is_object())
                        .map(playlist)
                        .filter(|playlist| !playlist.id.is_empty()),
                );
            },
        )
        .await?;
        Ok(result)
    }

    async fn playlist_songs(&self, id: &str, token: &str) -> Result<Vec<Song>> {
        let mut result = Vec::new();
        let query = [("limit", "50"), ("additional_types", "track"), MARKET];
        self.collect_offset_pages(
            &format!("/v1/playlists/{id}/items"),
            &query,
            token,
            |page| wrapped_tracks(&mut result, page),
        )
        .await?;
        Ok(result)
    }

    async fn saved_songs(&self, token: &str) -> Result<Vec<Song>> {
        let mut result = Vec::new();
        self.collect_offset_pages("/v1/me/tracks", &[MARKET], token, |page| {
            wrapped_tracks(&mut result, page)
        })
        .await?;
        Ok(result)
    }

    async fn saved_song_total(&self, token: &str) -> Result<i64> {
        let page = self
            .get(self.url("/v1/me/tracks", &[("limit", "1")]), token)
            .await?;
        Ok(page
            .get("total")
            .and_then(Value::as_i64)
            .unwrap_or_default())
    }

    async fn recently_played(&self, token: &str) -> Result<Vec<Song>> {
        let page = self
            .get(
                self.url("/v1/me/player/recently-played", &[("limit", "20")]),
                token,
            )
            .await?;
        let mut result = Vec::new();
        wrapped_tracks(&mut result, &page);
        Ok(result)
    }

    async fn search(&self, query: &str, token: &str) -> Result<SearchResults> {
        let url = self.url(
            "/v1/search",
            &[
                ("q", query),
                ("type", "track,album,artist,playlist"),
                ("limit", "10"),
                MARKET,
            ],
        );
        let response = self.get(url, token).await?;
        let items = |key: &str| {
            response
                .get(key)
                .map(|section| array(section, "items"))
                .unwrap_or_default()
        };
        Ok(SearchResults {
            songs: items("tracks")
                .iter()
                .filter_map(|value| track(value, &Album::default()))
                .collect(),
            albums: items("albums").iter().filter_map(album).collect(),
            artists: items("artists").iter().filter_map(artist).collect(),
            playlists: items("playlists")
                .iter()
                .filter(|value| value.is_object())
                .map(playlist)
                .filter(|playlist| !playlist.id.is_empty())
                .collect(),
        })
    }

    async fn song(&self, id: &str, token: &str) -> Result<Song> {
        let object = self
            .get(self.url(&format!("/v1/tracks/{id}"), &[MARKET]), token)
            .await?;
        track(&object, &Album::default())
            .ok_or_else(|| invalid_data("Spotify returned an unsupported track."))
    }

    async fn album(&self, id: &str, token: &str) -> Result<Album> {
        let object = self
            .get(self.url(&format!("/v1/albums/{id}"), &[MARKET]), token)
            .await?;
        album(&object).ok_or_else(|| invalid_data("Spotify returned an unsupported album."))
    }

    async fn artist(&self, id: &str, token: &str) -> Result<Artist> {
        let object = self
            .get(self.url(&format!("/v1/artists/{id}"), &[MARKET]), token)
            .await?;
        artist(&object).ok_or_else(|| invalid_data("Spotify returned an unsupported artist."))
    }

    async fn playlist(&self, id: &str, token: &str) -> Result<Playlist> {
        // playlist() reads both "items" and "tracks"
        let fields = "id,name,description,images,uri,public,collaborative,owner(id,display_name),items(total),tracks(total)";
        let object = self
            .get(
                self.url(
                    &format!("/v1/playlists/{id}"),
                    &[("fields", fields), MARKET],
                ),
                token,
            )
            .await?;
        let result = playlist(&object);
        if result.id.is_empty() {
            return Err(invalid_data("Spotify returned an unsupported playlist."));
        }
        Ok(result)
    }

    async fn album_songs(&self, album: &Album, token: &str) -> Result<Vec<Song>> {
        let mut result = Vec::new();
        let url = self.url(
            &format!("/v1/albums/{}/tracks", album.id),
            &[("limit", "50"), MARKET],
        );
        self.collect_pages(url, token, |page| {
            result.extend(
                array(page, "items")
                    .iter()
                    .filter_map(|value| track(value, album)),
            );
        })
        .await?;
        Ok(result)
    }

    async fn artist_albums(&self, artist: &Artist, token: &str) -> Result<Vec<Album>> {
        let mut result = Vec::new();
        let mut seen = HashSet::new();
        let url = self.url(
            &format!("/v1/artists/{}/albums", artist.id),
            &[("include_groups", "album,single"), ("limit", "10")],
        );
        self.collect_pages(url, token, |page| {
            for found in array(page, "items").iter().filter_map(album) {
                if seen.insert(found.id.clone()) {
                    result.push(found);
                }
            }
        })
        .await?;
        Ok(result)
    }

    async fn artist_genres(&self, ids: &[String], token: &str) -> Result<ArtistGenres> {
        let mut result = ArtistGenres::default();
        // fifty at a time is as many as spotify takes
        for chunk in ids.chunks(50) {
            let joined = chunk.join(",");
            let page = self
                .get(self.url("/v1/artists", &[("ids", &joined)]), token)
                .await?;
            for object in array(&page, "artists") {
                let id = string(object, "id");
                if id.is_empty() {
                    continue;
                }
                let genres = array(object, "genres")
                    .iter()
                    .map(|genre| genre.as_str().unwrap_or_default().to_owned());
                result.genres.insert(id.clone(), genres.collect());
                let image = preferred_artwork_url(array(object, "images"), 320);
                if !image.is_empty() {
                    result.artwork.insert(id, image);
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
        token: &str,
    ) -> Result<Playlist> {
        let body = json!({"name": name, "description": description, "public": is_public});
        let data = self
            .send_json(self.url("/v1/me/playlists", &[]), Method::POST, token, body)
            .await?;
        let created = playlist(
            &serde_json::from_slice(&data).map_err(|error| invalid_data(&error.to_string()))?,
        );
        if created.id.is_empty() {
            return Err(invalid_data("the new playlist had no id."));
        }
        Ok(created)
    }

    async fn update_playlist(
        &self,
        id: &str,
        name: &str,
        description: &str,
        is_public: bool,
        token: &str,
    ) -> Result<()> {
        let body = json!({"name": name, "description": description, "public": is_public});
        self.send_json(
            self.url(&format!("/v1/playlists/{id}"), &[]),
            Method::PUT,
            token,
            body,
        )
        .await?;
        Ok(())
    }

    async fn add_to_playlist(&self, uri: &str, playlist_id: &str, token: &str) -> Result<()> {
        let url = self.url(&format!("/v1/playlists/{playlist_id}/items"), &[]);
        self.send_json(url, Method::POST, token, json!({"uris": [uri]}))
            .await?;
        Ok(())
    }

    async fn remove_from_playlist(&self, uri: &str, playlist_id: &str, token: &str) -> Result<()> {
        let url = self.url(&format!("/v1/playlists/{playlist_id}/items"), &[]);
        self.send_json(url, Method::DELETE, token, json!({"items": [{"uri": uri}]}))
            .await?;
        Ok(())
    }

    async fn reorder_playlist(
        &self,
        playlist_id: &str,
        range_start: i64,
        insert_before: i64,
        length: i64,
        token: &str,
    ) -> Result<()> {
        let url = self.url(&format!("/v1/playlists/{playlist_id}/items"), &[]);
        let body = json!({"range_start": range_start, "insert_before": insert_before, "range_length": length});
        self.send_json(url, Method::PUT, token, body).await?;
        Ok(())
    }

    async fn set_playlist_image(
        &self,
        playlist_id: &str,
        jpeg_base64: &str,
        token: &str,
    ) -> Result<()> {
        // base64 grows by four thirds: the limit is on the image itself
        if jpeg_base64.len() / 4 * 3 > MAXIMUM_COVER_BYTES {
            return Err(request_failed("That cover image is too large."));
        }
        let url = self.url(&format!("/v1/playlists/{playlist_id}/images"), &[]);
        self.request_data(
            url,
            Method::PUT,
            token,
            Some((jpeg_base64.as_bytes().to_vec(), "image/jpeg")),
        )
        .await?;
        Ok(())
    }

    async fn delete_playlist(&self, id: &str, token: &str) -> Result<()> {
        self.set_saved(false, &format!("spotify:playlist:{id}"), token)
            .await
    }

    async fn set_saved(&self, saved: bool, uri: &str, token: &str) -> Result<()> {
        let method = if saved { Method::PUT } else { Method::DELETE };
        self.request_data(
            self.url("/v1/me/library", &[("uris", uri)]),
            method,
            token,
            None,
        )
        .await?;
        Ok(())
    }

    async fn saved_statuses(&self, uris: &[String], token: &str) -> Result<HashMap<String, bool>> {
        let mut unique: Vec<&String> = Vec::new();
        for uri in uris {
            if !unique.contains(&uri) {
                unique.push(uri);
            }
        }
        unique.truncate(40);
        let mut result = HashMap::new();
        if unique.is_empty() {
            return Ok(result);
        }
        let joined = unique
            .iter()
            .map(|uri| uri.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let statuses = self
            .get(
                self.url("/v1/me/library/contains", &[("uris", &joined)]),
                token,
            )
            .await?;
        let statuses = statuses.as_array().map(Vec::as_slice).unwrap_or_default();
        for (uri, status) in unique.iter().zip(statuses) {
            result.insert((*uri).clone(), status.as_bool().unwrap_or_default());
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptions_lose_their_html() {
        assert_eq!(
            plain_text("It&#x27;s <a href=\"x\">here</a> &amp; there"),
            "It's here & there"
        );
        assert_eq!(
            plain_text("&#39;&unknown; &#0; 1 < 2"),
            "'&unknown; &#0; 1 < 2"
        );
        assert_eq!(plain_text("a &nbsp;b&lt;"), "a  b<");
    }

    /// tst_session's aPlaylistDescriptionReadsAsPlainText, as it was
    #[test]
    fn a_playlist_description_reads_as_plain_text() {
        let description =
            |html: &str| playlist(&json!({"id": "x", "description": html})).description;
        assert_eq!(
            description("JoJo&#x27;s ops &amp; some other bangers."),
            "JoJo's ops & some other bangers."
        );
        assert_eq!(
            description(
                r#"more at <a href="spotify:playlist:y">Part 2</a> &#39;n &quot;more&quot;"#
            ),
            "more at Part 2 'n \"more\""
        );
        // not an entity, left as it is
        assert_eq!(
            description("rock &roll; &bogus; a&b"),
            "rock &roll; &bogus; a&b"
        );
    }

    /// tst_session's aTrackSaysWhetherItWillPlay, as it was
    #[test]
    fn a_track_says_whether_it_will_play() {
        let parse = |object: Value| track(&object, &Album::default()).unwrap();
        assert!(parse(json!({"type": "track", "uri": "spotify:track:a", "name": "A"})).is_playable);
        assert!(!parse(json!({"type": "track", "uri": "spotify:track:a", "name": "A", "is_playable": false})).is_playable);
        let relinked = parse(
            json!({"type": "track", "uri": "spotify:track:b", "name": "A", "is_playable": true,
                                    "linked_from": {"uri": "spotify:track:a"}}),
        );
        assert_eq!(relinked.uri, "spotify:track:a");
    }

    #[test]
    fn artwork_closest_to_the_width_wins_first_on_ties() {
        let images = json!([
            {"url": "https://a", "width": 300},
            {"url": "https://b", "width": 980},
            {"url": "https://c", "width": 300},
            {"url": "", "width": 640},
        ]);
        assert_eq!(
            preferred_artwork_url(images.as_array().unwrap(), 640),
            "https://a"
        );
        let no_width = json!([{"url": "https://z"}]);
        assert_eq!(
            preferred_artwork_url(no_width.as_array().unwrap(), 640),
            "https://z"
        );
    }

    #[test]
    fn tracks_take_the_relinked_uri_and_the_fallback_album() {
        let fallback = Album {
            id: "al".into(),
            name: "Album".into(),
            artwork_url: "https://cover".into(),
            uri: "spotify:album:al".into(),
            ..Album::default()
        };
        let object = json!({
            "type": "track", "uri": "spotify:track:copy", "name": "Song", "duration_ms": 1000,
            "linked_from": {"uri": "spotify:track:original"},
            "artists": [{"id": "ar", "uri": "spotify:artist:ar", "name": "One"}, {"name": "Two"}],
        });
        let song = track(&object, &fallback).unwrap();
        assert_eq!(song.uri, "spotify:track:original");
        assert_eq!(song.artists, "One, Two");
        assert_eq!(song.artist_items.len(), 1);
        assert_eq!(song.album_id, "al");
        assert_eq!(song.artwork_url, "https://cover");
        assert!(song.is_playable);
        assert!(
            track(
                &json!({"type": "episode", "uri": "spotify:track:x"}),
                &fallback
            )
            .is_none()
        );
        assert!(track(&json!({"uri": "spotify:episode:x"}), &fallback).is_none());
    }

    #[test]
    fn wrapped_tracks_read_item_then_track() {
        let item = json!({"item": {"uri": "spotify:track:a"}});
        let legacy = json!({"track": {"uri": "spotify:track:b"}});
        assert_eq!(wrapped_track(&item).unwrap().uri, "spotify:track:a");
        assert_eq!(wrapped_track(&legacy).unwrap().uri, "spotify:track:b");
        assert!(wrapped_track(&json!({"item": null})).is_none());
    }

    #[test]
    fn playlists_default_their_uri_and_count_items_first() {
        let object =
            json!({"id": "p", "items": {"total": 3}, "tracks": {"total": 9}, "owner": {"id": "o"}});
        let parsed = playlist(&object);
        assert_eq!(parsed.uri, "spotify:playlist:p");
        assert_eq!(parsed.song_count, 3);
        assert_eq!(parsed.owner_id, "o");
        assert_eq!(
            playlist(&json!({"id": "p", "tracks": {"total": 9}})).song_count,
            9
        );
    }

    #[test]
    fn profiles_without_a_name_get_one() {
        assert_eq!(profile(&json!({"id": "u"})).display_name, "Spotify User");
    }

    #[test]
    fn statuses_map_to_the_front_end_errors() {
        let unauthorized = status_error(401, br#"{"error":{"message":"expired"}}"#, None);
        assert_eq!(unauthorized.kind, ErrorKind::Unauthorized);
        assert_eq!(
            unauthorized.message,
            "Spotify API authorization failed: expired"
        );
        let limited = status_error(429, b"", Some(7));
        assert_eq!(
            (limited.kind, limited.retry_after),
            (ErrorKind::RateLimited, 7)
        );
        assert_eq!(
            status_error(500, b"", None).message,
            "Spotify API request failed: HTTP 500"
        );
    }

    #[test]
    fn urls_stay_on_the_api_host() {
        let url = api_url("/v1/search", &[("q", "a b"), MARKET]);
        assert_eq!(url.host_str(), Some("api.spotify.com"));
        assert!(url.as_str().contains("market=from_token"));
    }

    /// a stand-in for api.spotify.com: answers paths from `routes`, 404 with
    /// spotify's error body otherwise, and records each request line
    async fn fake_api(
        routes: Vec<(&'static str, &'static str)>,
    ) -> (Spotify, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = std::sync::Arc::clone(&calls);
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut request = Vec::new();
                let mut buffer = [0u8; 4096];
                while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                    let Ok(read) = stream.read(&mut buffer).await else {
                        break;
                    };
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..read]);
                }
                let text = String::from_utf8_lossy(&request).into_owned();
                let line = text.lines().next().unwrap_or_default().to_owned();
                let target = line.split(' ').nth(1).unwrap_or_default().to_owned();
                recorded.lock().unwrap().push(line);
                let path = target.split('?').next().unwrap_or_default();
                let (status, body) = match routes.iter().find(|(route, _)| *route == path) {
                    Some((_, body)) => ("200 OK", (*body).to_owned()),
                    None => (
                        "404 Not Found",
                        r#"{"error":{"status":404,"message":"Not found"}}"#.to_owned(),
                    ),
                };
                let reply = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes()).await;
            }
        });
        (Spotify::new(base), calls)
    }

    const ALBUM: &str = "/v1/albums/4aawyAB9vmqN3uQ7FjRGTy";
    const PLAYLIST: &str = "/v1/playlists/37i9dQZF1DXcBWIGoYBM5M";
    const ARTIST: &str = "/v1/artists/4Z8W4fKeB5YxbusRsdQVPb";

    #[tokio::test]
    async fn an_album_is_read_by_its_id() {
        let (api, calls) = fake_api(vec![(
            ALBUM,
            r#"{"id":"4aawyAB9vmqN3uQ7FjRGTy","uri":"spotify:album:4aawyAB9vmqN3uQ7FjRGTy","name":"Global Warming","artists":[{"name":"Pitbull"}],"images":[{"url":"https:\/\/img.test/cover","width":640}]}"#,
        )])
        .await;
        let album = api.album("4aawyAB9vmqN3uQ7FjRGTy", "access").await.unwrap();
        assert_eq!(album.id, "4aawyAB9vmqN3uQ7FjRGTy");
        assert_eq!(album.name, "Global Warming");
        assert_eq!(album.artists, "Pitbull");
        assert_eq!(album.artwork_url, "https://img.test/cover");
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].starts_with(&format!("GET {ALBUM}")));
    }

    #[tokio::test]
    async fn an_album_without_an_id_is_unsupported() {
        let (api, _) = fake_api(vec![(ALBUM, "{}")]).await;
        let failure = api
            .album("4aawyAB9vmqN3uQ7FjRGTy", "access")
            .await
            .unwrap_err();
        assert!(
            failure.message.contains("unsupported album"),
            "{}",
            failure.message
        );
    }

    #[tokio::test]
    async fn a_playlist_is_read_by_its_id_without_its_tracks() {
        let (api, calls) = fake_api(vec![(
            PLAYLIST,
            r#"{"id":"37i9dQZF1DXcBWIGoYBM5M","name":"Todays Top Hits","owner":{"id":"spotify","display_name":"Spotify"},"images":[{"url":"https:\/\/img.test/playlist","width":640}]}"#,
        )])
        .await;
        let playlist = api
            .playlist("37i9dQZF1DXcBWIGoYBM5M", "access")
            .await
            .unwrap();
        assert_eq!(playlist.name, "Todays Top Hits");
        assert_eq!(playlist.owner_id, "spotify");
        assert_eq!(playlist.owner_name, "Spotify");
        assert_eq!(playlist.artwork_url, "https://img.test/playlist");
        assert_eq!(playlist.uri, "spotify:playlist:37i9dQZF1DXcBWIGoYBM5M");
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].contains("fields="));
        assert!(!calls.join("\n").contains("/items"));
    }

    #[tokio::test]
    async fn a_playlist_without_an_id_is_unsupported() {
        let (api, _) = fake_api(vec![(PLAYLIST, "{}")]).await;
        let failure = api
            .playlist("37i9dQZF1DXcBWIGoYBM5M", "access")
            .await
            .unwrap_err();
        assert!(
            failure.message.contains("unsupported playlist"),
            "{}",
            failure.message
        );
    }

    #[tokio::test]
    async fn an_artist_is_read_by_its_id() {
        let (api, calls) = fake_api(vec![(
            ARTIST,
            r#"{"id":"4Z8W4fKeB5YxbusRsdQVPb","uri":"spotify:artist:4Z8W4fKeB5YxbusRsdQVPb","name":"Radiohead","images":[{"url":"https:\/\/img.test/artist","width":640}]}"#,
        )])
        .await;
        let artist = api
            .artist("4Z8W4fKeB5YxbusRsdQVPb", "access")
            .await
            .unwrap();
        assert_eq!(artist.id, "4Z8W4fKeB5YxbusRsdQVPb");
        assert_eq!(artist.name, "Radiohead");
        assert_eq!(artist.artwork_url, "https://img.test/artist");
        assert_eq!(calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn an_artist_without_an_id_is_unsupported() {
        let (api, _) = fake_api(vec![(ARTIST, "{}")]).await;
        let failure = api
            .artist("4Z8W4fKeB5YxbusRsdQVPb", "access")
            .await
            .unwrap_err();
        assert!(
            failure.message.contains("unsupported artist"),
            "{}",
            failure.message
        );
    }

    #[tokio::test]
    async fn a_missing_album_playlist_or_artist_results_in_the_not_found() {
        let (api, _) = fake_api(vec![]).await;
        let id = "4aawyAB9vmqN3uQ7FjRGTy";
        assert!(
            !api.album(id, "access")
                .await
                .unwrap_err()
                .message
                .is_empty()
        );
        assert!(
            !api.playlist(id, "access")
                .await
                .unwrap_err()
                .message
                .is_empty()
        );
        let artist = api.artist(id, "access").await.unwrap_err();
        assert_eq!(artist.message, "Spotify API request failed: Not found");
    }

    #[tokio::test]
    async fn another_host_is_refused() {
        let (api, _) = fake_api(vec![]).await;
        let failure = api
            .get(Url::parse("https://example.com/v1/me").unwrap(), "access")
            .await
            .unwrap_err();
        assert_eq!(failure.kind, ErrorKind::InvalidUrl);
    }
}
