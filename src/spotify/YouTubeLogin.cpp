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

#include "spotify/YouTubeLogin.h"

#include <QHash>

#ifdef SWIFTIFY_HAS_WEBENGINE
#include <QApplication>
#include <QCloseEvent>
#include <QNetworkCookie>
#include <QWebEngineCookieStore>
#include <QWebEnginePage>
#include <QWebEngineProfile>
#include <QWebEngineView>
#endif

namespace swiftify {

namespace {
/// where google's sign-in sends the account on to (Metrolist's LoginDialog)
constexpr auto kSignInUrl = "https://accounts.google.com/ServiceLogin?continue=https%3A%2F%2Fmusic.youtube.com";
} // namespace

QString youTubeCookieHeader(const QList<std::pair<QString, QString>> &cookies)
{
    QHash<QString, QString> latest;
    QStringList order;
    for (const auto &[name, value] : cookies) {
        if (!latest.contains(name)) {
            order.append(name);
        }
        latest.insert(name, value);
    }
    if (!latest.contains(QStringLiteral("SAPISID")) && !latest.contains(QStringLiteral("__Secure-3PAPISID"))) {
        return {};
    }
    QStringList pairs;
    for (const QString &name : std::as_const(order)) {
        pairs.append(name + u'=' + latest.value(name));
    }
    return pairs.join(QStringLiteral("; "));
}

#ifdef SWIFTIFY_HAS_WEBENGINE

struct YouTubeLogin::Window : QWebEngineView {
    QWebEngineProfile profile;
    QList<std::pair<QString, QString>> cookies;
    YouTubeLogin *owner;
    bool finished = false;

    explicit Window(YouTubeLogin *login)
        : owner(login)
    {
        setWindowTitle(QObject::tr("Sign in to YouTube Music"));
        resize(480, 720);
        // a profile without a storage name is off the record: in memory only.
        // the window keeps QtWebEngine's own user agent: google turns away an
        // engine passing for another (a firefox one got a captcha, a chrome one
        // "couldn't sign you in"), where QtWebEngine as itself got through
        setPage(new QWebEnginePage(&profile, this));
        QObject::connect(profile.cookieStore(), &QWebEngineCookieStore::cookieAdded, this,
                         [this](const QNetworkCookie &cookie) {
                             if (cookie.domain().endsWith(QLatin1StringView("youtube.com"))) {
                                 cookies.append({QString::fromUtf8(cookie.name()), QString::fromUtf8(cookie.value())});
                             }
                         });
        // the account is in once google has sent it on to youtube music with its cookies
        QObject::connect(page(), &QWebEnginePage::loadFinished, this, [this](bool) {
            if (finished || url().host() != QLatin1StringView("music.youtube.com")) {
                return;
            }
            const QString header = youTubeCookieHeader(cookies);
            if (header.isEmpty()) {
                return;
            }
            finished = true;
            emit owner->signedIn(header);
            close();
        });
        load(QUrl(QString::fromLatin1(kSignInUrl)));
    }

    ~Window() override
    {
        // the page goes before the profile it was made from
        delete page();
    }

protected:
    void closeEvent(QCloseEvent *event) override
    {
        if (!finished) {
            emit owner->cancelled();
        }
        event->accept();
        owner->m_window = nullptr;
        deleteLater();
    }
};

YouTubeLogin::YouTubeLogin(QObject *parent)
    : QObject(parent)
{
}

YouTubeLogin::~YouTubeLogin()
{
    delete m_window;
}

bool YouTubeLogin::isAvailable()
{
    return qobject_cast<QApplication *>(QCoreApplication::instance()) != nullptr;
}

void YouTubeLogin::open()
{
    if (!isAvailable()) {
        emit cancelled();
        return;
    }
    if (!m_window) {
        m_window = new Window(this);
    }
    m_window->show();
    m_window->raise();
    m_window->activateWindow();
}

#else

struct YouTubeLogin::Window {};

YouTubeLogin::YouTubeLogin(QObject *parent)
    : QObject(parent)
{
}

YouTubeLogin::~YouTubeLogin() = default;

bool YouTubeLogin::isAvailable()
{
    return false;
}

void YouTubeLogin::open()
{
    emit cancelled();
}

#endif

} // namespace swiftify
