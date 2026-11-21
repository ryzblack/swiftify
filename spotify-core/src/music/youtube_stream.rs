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

//! youtube music's audio, straight from innertube. youtube hands direct,
//! unsigned stream urls to few clients now; the visionOS client is one of them
//! (MetrolistGroup/innertubex's PlaybackClientCatalog, VISIONOS_0_1: "fastest
//! client for normal and non-age-gated explicit audio, signed in and out",
//! October 2026), given the visitor data youtube's own pages carry. no
//! signature to decipher, no proof-of-origin token. when youtube changes
//! clients again, this file is the one to change.

use std::path::Path;

use serde_json::{Value, json};

/// one innertube client, as youtube's player knows it
pub struct ClientProfile {
    pub name: &'static str,
    pub version: &'static str,
    /// X-YouTube-Client-Name
    pub id: &'static str,
    pub user_agent: &'static str,
    pub os_name: &'static str,
    pub os_version: &'static str,
    pub device_make: &'static str,
    pub device_model: &'static str,
    pub platform: &'static str,
}

pub const VISIONOS: ClientProfile = ClientProfile {
    name: "VISIONOS",
    version: "0.1",
    id: "101",
    user_agent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_6) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Safari/605.1.15",
    os_name: "VISION_OS",
    os_version: "1.3",
    device_make: "Apple",
    device_model: "RealityDevice14,1",
    platform: "MOBILE",
};

const PLAYER: &str = "https://music.youtube.com/youtubei/v1/player?prettyPrint=false";
const ORIGIN: &str = "https://music.youtube.com";
/// googlevideo slows a stream fetched in one request down to playback speed
const CHUNK: u64 = 9 * 1024 * 1024;

#[derive(Debug, PartialEq)]
pub enum StreamError {
    /// youtube will not play it to this client (age-gated, made for kids, a
    /// region it is not in): its reason
    Unplayable(String),
    /// the request itself failed
    Failed(String),
}

#[derive(Debug, PartialEq)]
pub struct Stream {
    pub url: String,
    pub size: u64,
}

/// the aac audio in an mp4 to play: itag 140 (128 kbit/s) when there, else the
/// best-rated aac with a direct url. opus comes in webm, which the core's
/// decoder does not read
pub fn pick_audio(player: &Value) -> Result<Stream, StreamError> {
    let status = player
        .pointer("/playabilityStatus/status")
        .and_then(Value::as_str)
        .unwrap_or("UNKNOWN");
    if status != "OK" {
        let reason = player
            .pointer("/playabilityStatus/reason")
            .and_then(Value::as_str)
            .unwrap_or(status);
        return Err(StreamError::Unplayable(reason.to_owned()));
    }
    let formats = player
        .pointer("/streamingData/adaptiveFormats")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let aac = formats.iter().filter(|format| {
        format.get("url").and_then(Value::as_str).is_some()
            && format
                .get("mimeType")
                .and_then(Value::as_str)
                .is_some_and(|mime| mime.starts_with("audio/mp4"))
    });
    let chosen = aac
        .clone()
        .find(|format| format.get("itag").and_then(Value::as_u64) == Some(140))
        .or_else(|| aac.max_by_key(|format| format.get("bitrate").and_then(Value::as_u64)))
        .ok_or_else(|| StreamError::Unplayable("no audio this player can decode".to_owned()))?;
    Ok(Stream {
        url: chosen
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        size: chosen
            .get("contentLength")
            .and_then(Value::as_str)
            .and_then(|length| length.parse().ok())
            .unwrap_or(0),
    })
}

/// the song's audio stream, as `client` is given it
pub async fn resolve(
    http: &reqwest::Client,
    client: &ClientProfile,
    video_id: &str,
    visitor_data: &str,
) -> Result<Stream, StreamError> {
    let body = json!({
        "context": {"client": {
            "clientName": client.name,
            "clientVersion": client.version,
            "osName": client.os_name,
            "osVersion": client.os_version,
            "deviceMake": client.device_make,
            "deviceModel": client.device_model,
            "platform": client.platform,
            "hl": "en",
            "gl": "US",
            "visitorData": visitor_data,
        }},
        "videoId": video_id,
        "contentCheckOk": true,
        "racyCheckOk": true,
    });
    let failed = |error: reqwest::Error| StreamError::Failed(error.to_string());
    let player: Value = http
        .post(PLAYER)
        .header(reqwest::header::USER_AGENT, client.user_agent)
        .header(reqwest::header::ORIGIN, ORIGIN)
        .header(reqwest::header::REFERER, format!("{ORIGIN}/"))
        .header("X-Goog-Visitor-Id", visitor_data)
        .header("X-YouTube-Client-Name", client.id)
        .header("X-YouTube-Client-Version", client.version)
        .json(&body)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(failed)?
        .json()
        .await
        .map_err(failed)?;
    pick_audio(&player)
}

/// the stream into `path`, a range at a time, with the user agent it was made for
pub async fn download(
    http: &reqwest::Client,
    client: &ClientProfile,
    stream: &Stream,
    path: &Path,
) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;
    let mut file = tokio::fs::File::create(path)
        .await
        .map_err(|failure| format!("could not write the YouTube audio: {failure}"))?;
    let mut offset = 0;
    loop {
        let end = offset + CHUNK - 1;
        let end = if stream.size > 0 {
            end.min(stream.size - 1)
        } else {
            end
        };
        let response = http
            .get(&stream.url)
            .header(reqwest::header::RANGE, format!("bytes={offset}-{end}"))
            .header(reqwest::header::USER_AGENT, client.user_agent)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|failure| format!("could not download the YouTube audio: {failure}"))?;
        let bytes = response
            .bytes()
            .await
            .map_err(|failure| format!("the YouTube audio stopped downloading: {failure}"))?;
        file.write_all(&bytes)
            .await
            .map_err(|failure| format!("could not write the YouTube audio: {failure}"))?;
        offset += bytes.len() as u64;
        let done = if stream.size > 0 {
            offset >= stream.size
        } else {
            (bytes.len() as u64) < CHUNK
        };
        if done || bytes.is_empty() {
            break;
        }
    }
    file.flush()
        .await
        .map_err(|failure| format!("could not write the YouTube audio: {failure}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format(itag: u64, mime: &str, bitrate: u64, url: bool) -> Value {
        let mut format =
            json!({"itag": itag, "mimeType": mime, "bitrate": bitrate, "contentLength": "1000"});
        if url {
            format["url"] = json!(format!("https://g/{itag}"));
        } else {
            format["signatureCipher"] = json!("s=x");
        }
        format
    }

    #[test]
    fn itag_140_is_chosen_first() {
        let player = json!({"playabilityStatus": {"status": "OK"}, "streamingData": {"adaptiveFormats": [
            format(251, "audio/webm; codecs=\"opus\"", 160_000, true),
            format(141, "audio/mp4; codecs=\"mp4a.40.2\"", 256_000, true),
            format(140, "audio/mp4; codecs=\"mp4a.40.2\"", 128_000, true),
        ]}});
        assert_eq!(
            pick_audio(&player).unwrap(),
            Stream {
                url: "https://g/140".into(),
                size: 1000
            }
        );
    }

    #[test]
    fn the_best_direct_aac_stands_in_for_140() {
        let player = json!({"playabilityStatus": {"status": "OK"}, "streamingData": {"adaptiveFormats": [
            format(139, "audio/mp4; codecs=\"mp4a.40.5\"", 48_000, true),
            format(141, "audio/mp4; codecs=\"mp4a.40.2\"", 256_000, false),
            format(251, "audio/webm; codecs=\"opus\"", 160_000, true),
        ]}});
        assert_eq!(pick_audio(&player).unwrap().url, "https://g/139");
    }

    #[test]
    fn unplayable_songs_say_why() {
        let player = json!({"playabilityStatus": {"status": "LOGIN_REQUIRED", "reason": "Sign in to confirm your age"}});
        assert_eq!(
            pick_audio(&player),
            Err(StreamError::Unplayable(
                "Sign in to confirm your age".into()
            ))
        );
        let ciphered = json!({"playabilityStatus": {"status": "OK"}, "streamingData": {"adaptiveFormats": [
            format(140, "audio/mp4", 128_000, false),
        ]}});
        assert!(matches!(
            pick_audio(&ciphered),
            Err(StreamError::Unplayable(_))
        ));
    }
}
