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

#include <QObject>
#include <QString>

namespace swiftify {

/// signing in to youtube music in a browser window of the app's own, as
/// Metrolist's desktop app does: youtube's account is its google cookies, and
/// a browser outside the app keeps them to itself. the window's profile is
/// off the record, so nothing of the sign-in stays on disk; the cookies it
/// ends with go to the session, which keeps them in the keychain
class YouTubeLogin : public QObject {
    Q_OBJECT

public:
    explicit YouTubeLogin(QObject *parent = nullptr);
    ~YouTubeLogin() override;

    /// built with QtWebEngine, and running with widgets (not on macos)
    static bool isAvailable();

    /// opens the window, or raises it when it is open already
    void open();

signals:
    /// the Cookie header of the signed-in account
    void signedIn(const QString &cookies);
    /// the window was closed before the sign-in finished
    void cancelled();

private:
    struct Window;
    Window *m_window = nullptr;
};

/// the Cookie header for youtube's api from cookies seen in the browser,
/// once they hold an account's (SAPISID): empty until then
QString youTubeCookieHeader(const QList<std::pair<QString, QString>> &cookies);

} // namespace swiftify
