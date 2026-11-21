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

#pragma once

#include "spotify/SpotifyError.h"
#include "spotify/SpotifyTypes.h"

#include <QByteArray>
#include <QHash>
#include <QJsonObject>
#include <QJsonValue>

#include <QCoroTask>

#include <functional>

namespace swiftify {

class PlaybackCore;

/// the music service the library reads from: spotify's web api or youtube
/// music. the calls are answered in the rust core (spotify-core src/music),
/// which shapes every service's answers as these types; this class only
/// carries them across. the access token is spotify's and is ignored by
/// youtube music, which keeps its own login
class WebApi {
public:
    static constexpr int kMaximumCoverBytes = 256'000;
    static inline const QString kSpotify = QStringLiteral("spotify");
    static inline const QString kYouTube = QStringLiteral("youtube");

    explicit WebApi(PlaybackCore *core);

    /// which service the calls below go to
    QString service() const { return m_service; }
    void setService(const QString &service) { m_service = service; }
    bool isYouTube() const { return m_service == kYouTube; }
    /// what a service keeps between runs changed (youtube's login): the
    /// caller keeps it in the keychain
    void setStateHandler(std::function<void(const QString &service, const QString &state)> handler)
    {
        m_onState = std::move(handler);
    }
    /// every failed call, before it is thrown on (a login a service stopped taking)
    void setFailureHandler(std::function<void(const QString &service, const SpotifyError &error)> handler)
    {
        m_onFailure = std::move(handler);
    }

    QCoro::Task<Profile> profile(QString accessToken);
    QCoro::Task<QList<Playlist>> playlists(QString accessToken);
    QCoro::Task<QList<Song>> playlistSongs(QString playlistId, QString accessToken);
    QCoro::Task<QList<Song>> savedSongs(QString accessToken);
    QCoro::Task<int> savedSongTotal(QString accessToken);
    QCoro::Task<QList<Song>> recentlyPlayed(QString accessToken);
    QCoro::Task<SearchResults> search(QString query, QString accessToken);
    QCoro::Task<Song> song(QString trackId, QString accessToken);
    QCoro::Task<Album> album(QString albumId, QString accessToken);
    QCoro::Task<Playlist> playlist(QString playlistId, QString accessToken);
    QCoro::Task<Artist> artist(QString artistId, QString accessToken);
    QCoro::Task<QList<Song>> albumSongs(Album album, QString accessToken);
    QCoro::Task<QList<Album>> artistAlbums(Artist artist, QString accessToken);
    QCoro::Task<QHash<QString, QStringList>> artistGenres(QStringList artistIds, QString accessToken,
                                                         QHash<QString, QUrl> *artwork = nullptr);
    QCoro::Task<Playlist> createPlaylist(QString name, QString description, bool isPublic, QString accessToken);
    QCoro::Task<> updatePlaylist(QString id, QString name, QString description, bool isPublic, QString accessToken);
    QCoro::Task<> addToPlaylist(QString songUri, QString playlistId, QString accessToken);
    QCoro::Task<> removeFromPlaylist(QString songUri, QString playlistId, QString accessToken);
    QCoro::Task<> reorderPlaylist(QString playlistId, int rangeStart, int insertBefore, int length,
                                  QString accessToken);
    QCoro::Task<> setPlaylistImage(QString playlistId, QByteArray jpeg, QString accessToken);
    QCoro::Task<> deletePlaylist(QString id, QString accessToken);
    QCoro::Task<> setSaved(bool saved, QString uri, QString accessToken);
    QCoro::Task<QHash<QString, bool>> savedStatuses(QStringList uris, QString accessToken);

    /// a call on the active service, or on the one named
    QCoro::Task<QJsonValue> call(QString method, QJsonObject args = {}, QString accessToken = {});
    QCoro::Task<QJsonValue> callOn(QString service, QString method, QJsonObject args = {}, QString accessToken = {});

    static Album albumFromJson(const QJsonObject &object);
    static Playlist playlistFromJson(const QJsonObject &object);
    static QList<Song> songsFromJson(const QJsonValue &value);

private:
    PlaybackCore *m_core;
    QString m_service = kSpotify;
    std::function<void(const QString &, const QString &)> m_onState;
    std::function<void(const QString &, const SpotifyError &)> m_onFailure;
};

} // namespace swiftify
