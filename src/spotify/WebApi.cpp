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

#include "spotify/WebApi.h"

#include "core/PlaybackCore.h"
#include "spotify/SpotifyError.h"

#include <QJsonArray>
#include <QJsonDocument>

namespace swiftify {

namespace {

/// a service's failure, as the core sends it ({"kind", "message", "retryAfter"})
SpotifyError serviceError(const QString &payload)
{
    static const QHash<QString, SpotifyError::Kind> kinds {
        {QStringLiteral("InvalidUrl"), SpotifyError::Kind::InvalidUrl},
        {QStringLiteral("InvalidData"), SpotifyError::Kind::InvalidData},
        {QStringLiteral("Unauthorized"), SpotifyError::Kind::Unauthorized},
        {QStringLiteral("Forbidden"), SpotifyError::Kind::Forbidden},
        {QStringLiteral("RateLimited"), SpotifyError::Kind::RateLimited},
        {QStringLiteral("RequestFailed"), SpotifyError::Kind::RequestFailed},
        {QStringLiteral("LoginRevoked"), SpotifyError::Kind::LoginRevoked},
        {QStringLiteral("Unavailable"), SpotifyError::Kind::Unavailable},
    };
    const QJsonObject error = QJsonDocument::fromJson(payload.toUtf8()).object();
    if (!error.contains(QStringLiteral("kind"))) {
        return SpotifyError::requestFailed(payload);
    }
    return {kinds.value(error.value(QStringLiteral("kind")).toString(), SpotifyError::Kind::RequestFailed),
            error.value(QStringLiteral("message")).toString(), error.value(QStringLiteral("retryAfter")).toInt(-1)};
}

Profile profileFromJson(const QJsonObject &object)
{
    return {
        object.value(QStringLiteral("id")).toString(),
        object.value(QStringLiteral("displayName")).toString(),
        QUrl(object.value(QStringLiteral("artworkURL")).toString()),
    };
}

template <typename T, typename Read>
QList<T> listFromJson(const QJsonValue &value, Read read)
{
    QList<T> result;
    for (const QJsonValue &item : value.toArray()) {
        result.append(read(item.toObject()));
    }
    return result;
}

} // namespace

WebApi::WebApi(PlaybackCore *core)
    : m_core(core)
{
}

Album WebApi::albumFromJson(const QJsonObject &object)
{
    Album album;
    album.id = object.value(QStringLiteral("id")).toString();
    album.name = object.value(QStringLiteral("name")).toString();
    album.artists = object.value(QStringLiteral("artists")).toString();
    album.artworkUrl = QUrl(object.value(QStringLiteral("artworkURL")).toString());
    album.releaseDate = object.value(QStringLiteral("releaseDate")).toString();
    album.songCount = object.value(QStringLiteral("songCount")).toInt();
    album.uri = object.value(QStringLiteral("uri")).toString();
    return album;
}

Playlist WebApi::playlistFromJson(const QJsonObject &object)
{
    Playlist playlist;
    playlist.id = object.value(QStringLiteral("id")).toString();
    playlist.name = object.value(QStringLiteral("name")).toString();
    playlist.description = object.value(QStringLiteral("description")).toString();
    playlist.songCount = object.value(QStringLiteral("songCount")).toInt();
    playlist.artworkUrl = QUrl(object.value(QStringLiteral("artworkURL")).toString());
    playlist.ownerId = object.value(QStringLiteral("ownerId")).toString();
    playlist.ownerName = object.value(QStringLiteral("ownerName")).toString();
    playlist.isPublic = object.value(QStringLiteral("isPublic")).toBool();
    playlist.isCollaborative = object.value(QStringLiteral("isCollaborative")).toBool();
    playlist.uri = object.value(QStringLiteral("uri")).toString();
    return playlist;
}

QList<Song> WebApi::songsFromJson(const QJsonValue &value)
{
    return listFromJson<Song>(value, Song::fromJson);
}

QCoro::Task<QJsonValue> WebApi::call(QString method, QJsonObject args, QString accessToken)
{
    co_return co_await callOn(m_service, std::move(method), std::move(args), std::move(accessToken));
}

QCoro::Task<QJsonValue> WebApi::callOn(QString service, QString method, QJsonObject args, QString accessToken)
{
    const QByteArray body = QJsonDocument(args).toJson(QJsonDocument::Compact);
    QString payload;
    try {
        payload = co_await m_core->serviceCall(service, method, body, accessToken);
    } catch (const SpotifyError &error) {
        const SpotifyError failure = error.kind() == SpotifyError::Kind::Core ? serviceError(error.message()) : error;
        if (m_onFailure) {
            m_onFailure(service, failure);
        }
        throw failure;
    }
    QJsonParseError parseError;
    const QJsonObject envelope = QJsonDocument::fromJson(payload.toUtf8(), &parseError).object();
    if (parseError.error != QJsonParseError::NoError) {
        throw SpotifyError::invalidData(parseError.errorString());
    }
    if (const QJsonValue state = envelope.value(QStringLiteral("state")); state.isString() && m_onState) {
        m_onState(service, state.toString());
    }
    co_return envelope.value(QStringLiteral("value"));
}

QCoro::Task<Profile> WebApi::profile(QString accessToken)
{
    co_return profileFromJson((co_await call(QStringLiteral("profile"), {}, accessToken)).toObject());
}

QCoro::Task<QList<Playlist>> WebApi::playlists(QString accessToken)
{
    co_return listFromJson<Playlist>(co_await call(QStringLiteral("playlists"), {}, accessToken), playlistFromJson);
}

QCoro::Task<QList<Song>> WebApi::playlistSongs(QString playlistId, QString accessToken)
{
    co_return songsFromJson(co_await call(QStringLiteral("playlistSongs"), {{QStringLiteral("id"), playlistId}}, accessToken));
}

QCoro::Task<QList<Song>> WebApi::savedSongs(QString accessToken)
{
    co_return songsFromJson(co_await call(QStringLiteral("savedSongs"), {}, accessToken));
}

QCoro::Task<int> WebApi::savedSongTotal(QString accessToken)
{
    co_return (co_await call(QStringLiteral("savedSongTotal"), {}, accessToken)).toInt();
}

QCoro::Task<QList<Song>> WebApi::recentlyPlayed(QString accessToken)
{
    co_return songsFromJson(co_await call(QStringLiteral("recentlyPlayed"), {}, accessToken));
}

QCoro::Task<SearchResults> WebApi::search(QString query, QString accessToken)
{
    const QJsonObject found = (co_await call(QStringLiteral("search"), {{QStringLiteral("query"), query}}, accessToken)).toObject();
    SearchResults results;
    results.songs = songsFromJson(found.value(QStringLiteral("songs")));
    results.albums = listFromJson<Album>(found.value(QStringLiteral("albums")), albumFromJson);
    results.artists = listFromJson<Artist>(found.value(QStringLiteral("artists")), Artist::fromJson);
    results.playlists = listFromJson<Playlist>(found.value(QStringLiteral("playlists")), playlistFromJson);
    co_return results;
}

QCoro::Task<Song> WebApi::song(QString trackId, QString accessToken)
{
    co_return Song::fromJson((co_await call(QStringLiteral("song"), {{QStringLiteral("id"), trackId}}, accessToken)).toObject());
}

QCoro::Task<Album> WebApi::album(QString albumId, QString accessToken)
{
    co_return albumFromJson((co_await call(QStringLiteral("album"), {{QStringLiteral("id"), albumId}}, accessToken)).toObject());
}

QCoro::Task<Playlist> WebApi::playlist(QString playlistId, QString accessToken)
{
    co_return playlistFromJson(
        (co_await call(QStringLiteral("playlist"), {{QStringLiteral("id"), playlistId}}, accessToken)).toObject());
}

QCoro::Task<Artist> WebApi::artist(QString artistId, QString accessToken)
{
    co_return Artist::fromJson((co_await call(QStringLiteral("artist"), {{QStringLiteral("id"), artistId}}, accessToken)).toObject());
}

QCoro::Task<QList<Song>> WebApi::albumSongs(Album album, QString accessToken)
{
    const QJsonObject albumJson {
        {QStringLiteral("id"), album.id},
        {QStringLiteral("name"), album.name},
        {QStringLiteral("artists"), album.artists},
        {QStringLiteral("artworkURL"), album.artworkUrl.toString()},
        {QStringLiteral("uri"), album.uri},
    };
    co_return songsFromJson(co_await call(QStringLiteral("albumSongs"), {{QStringLiteral("album"), albumJson}}, accessToken));
}

QCoro::Task<QList<Album>> WebApi::artistAlbums(Artist artist, QString accessToken)
{
    co_return listFromJson<Album>(
        co_await call(QStringLiteral("artistAlbums"), {{QStringLiteral("artist"), artist.toJson()}}, accessToken), albumFromJson);
}

QCoro::Task<QHash<QString, QStringList>> WebApi::artistGenres(QStringList artistIds, QString accessToken,
                                                              QHash<QString, QUrl> *artwork)
{
    const QJsonObject found = (co_await call(QStringLiteral("artistGenres"),
                                             {{QStringLiteral("ids"), QJsonArray::fromStringList(artistIds)}}, accessToken))
                                  .toObject();
    QHash<QString, QStringList> result;
    const QJsonObject genres = found.value(QStringLiteral("genres")).toObject();
    for (auto it = genres.begin(); it != genres.end(); ++it) {
        QStringList names;
        for (const QJsonValue &genre : it.value().toArray()) {
            names.append(genre.toString());
        }
        result.insert(it.key(), names);
    }
    if (artwork) {
        const QJsonObject images = found.value(QStringLiteral("artwork")).toObject();
        for (auto it = images.begin(); it != images.end(); ++it) {
            artwork->insert(it.key(), QUrl(it.value().toString()));
        }
    }
    co_return result;
}

QCoro::Task<Playlist> WebApi::createPlaylist(QString name, QString description, bool isPublic, QString accessToken)
{
    co_return playlistFromJson((co_await call(QStringLiteral("createPlaylist"),
                                              {{QStringLiteral("name"), name},
                                               {QStringLiteral("description"), description},
                                               {QStringLiteral("isPublic"), isPublic}},
                                              accessToken))
                                   .toObject());
}

QCoro::Task<> WebApi::updatePlaylist(QString id, QString name, QString description, bool isPublic, QString accessToken)
{
    co_await call(QStringLiteral("updatePlaylist"),
                  {{QStringLiteral("id"), id},
                   {QStringLiteral("name"), name},
                   {QStringLiteral("description"), description},
                   {QStringLiteral("isPublic"), isPublic}},
                  accessToken);
}

QCoro::Task<> WebApi::addToPlaylist(QString songUri, QString playlistId, QString accessToken)
{
    co_await call(QStringLiteral("addToPlaylist"), {{QStringLiteral("uri"), songUri}, {QStringLiteral("playlistId"), playlistId}},
                  accessToken);
}

QCoro::Task<> WebApi::removeFromPlaylist(QString songUri, QString playlistId, QString accessToken)
{
    co_await call(QStringLiteral("removeFromPlaylist"),
                  {{QStringLiteral("uri"), songUri}, {QStringLiteral("playlistId"), playlistId}}, accessToken);
}

QCoro::Task<> WebApi::reorderPlaylist(QString playlistId, int rangeStart, int insertBefore, int length, QString accessToken)
{
    co_await call(QStringLiteral("reorderPlaylist"),
                  {{QStringLiteral("playlistId"), playlistId},
                   {QStringLiteral("rangeStart"), rangeStart},
                   {QStringLiteral("insertBefore"), insertBefore},
                   {QStringLiteral("length"), length}},
                  accessToken);
}

QCoro::Task<> WebApi::setPlaylistImage(QString playlistId, QByteArray jpeg, QString accessToken)
{
    if (jpeg.size() > kMaximumCoverBytes) {
        throw SpotifyError::requestFailed(QStringLiteral("That cover image is too large."));
    }
    co_await call(QStringLiteral("setPlaylistImage"),
                  {{QStringLiteral("playlistId"), playlistId}, {QStringLiteral("jpeg"), QString::fromLatin1(jpeg.toBase64())}},
                  accessToken);
}

QCoro::Task<> WebApi::deletePlaylist(QString id, QString accessToken)
{
    co_await call(QStringLiteral("deletePlaylist"), {{QStringLiteral("id"), id}}, accessToken);
}

QCoro::Task<> WebApi::setSaved(bool saved, QString uri, QString accessToken)
{
    co_await call(QStringLiteral("setSaved"), {{QStringLiteral("saved"), saved}, {QStringLiteral("uri"), uri}}, accessToken);
}

QCoro::Task<QHash<QString, bool>> WebApi::savedStatuses(QStringList uris, QString accessToken)
{
    uris.removeDuplicates();
    uris = uris.mid(0, 40);
    QHash<QString, bool> result;
    if (uris.isEmpty()) {
        co_return result;
    }
    const QJsonObject statuses
        = (co_await call(QStringLiteral("savedStatuses"), {{QStringLiteral("uris"), QJsonArray::fromStringList(uris)}}, accessToken))
              .toObject();
    for (auto it = statuses.begin(); it != statuses.end(); ++it) {
        result.insert(it.key(), it.value().toBool());
    }
    co_return result;
}

} // namespace swiftify
