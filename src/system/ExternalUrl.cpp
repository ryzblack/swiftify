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

#include "system/ExternalUrl.h"

#include <QDesktopServices>
#include <QLoggingCategory>

#ifdef SWIFTIFY_HAS_DBUS
#include <QDBusConnection>
#include <QDBusMessage>
#include <QDBusPendingCallWatcher>
#include <QDBusPendingReply>
#include <QVariantMap>
#endif

namespace swiftify {

namespace {
Q_LOGGING_CATEGORY(lcExternal, "swiftify.external")
} // namespace

void openExternally(const QUrl &url)
{
#ifdef SWIFTIFY_HAS_DBUS
    // the portal takes web addresses; a file needs a descriptor (OpenFile)
    const bool web = url.scheme() == QLatin1StringView("https") || url.scheme() == QLatin1StringView("http");
    if (web && qEnvironmentVariableIsSet("FLATPAK_ID")) {
        QDBusMessage call = QDBusMessage::createMethodCall(
            QStringLiteral("org.freedesktop.portal.Desktop"), QStringLiteral("/org/freedesktop/portal/desktop"),
            QStringLiteral("org.freedesktop.portal.OpenURI"), QStringLiteral("OpenURI"));
        call << QString() << url.toString(QUrl::FullyEncoded) << QVariantMap {};
        auto *watcher = new QDBusPendingCallWatcher(QDBusConnection::sessionBus().asyncCall(call));
        QObject::connect(watcher, &QDBusPendingCallWatcher::finished, watcher, [url](QDBusPendingCallWatcher *done) {
            done->deleteLater();
            if (done->isError()) {
                qCWarning(lcExternal).noquote() << "the desktop portal did not open" << url.host() << ':'
                                                << done->error().message();
                QDesktopServices::openUrl(url);
            }
        });
        return;
    }
#endif
    QDesktopServices::openUrl(url);
}

} // namespace swiftify
