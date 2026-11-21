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

pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls.Basic
import Swiftify

// connecting the library and playback for the first time (SpotifySetupView),
// or choosing youtube music, which needs no setup beyond an optional sign-in
Flickable {
    id: root

    /// the service being set up, chosen first
    property bool youTube: Session.musicService === "youtube"
    /// past the choice, on the chosen service's way in
    property bool chosen: false

    // pages run up under the top bar, which floats over them with no
    // background of its own; they start below it
    topMargin: Appearance.sizes.topBarHeight

    contentHeight: Math.max(height, column.implicitHeight + 72)
    boundsBehavior: Flickable.StopAtBounds
    ScrollBar.vertical: StyledScrollBar { topPadding: 3 + Appearance.sizes.topBarHeight }

    component Step: Row {
        id: step
        property int number
        property string title
        property string detail

        spacing: 12
        width: parent ? parent.width : 560

        Rectangle {
            width: 26
            height: 26
            radius: 13
            color: Appearance.colors.primary

            StyledText {
                anchors.centerIn: parent
                text: step.number
                font.weight: Font.Bold
                font.pixelSize: Appearance.fontSize.smallie
                color: Appearance.colors.onPrimary
            }
        }

        Column {
            width: step.width - 38
            spacing: 2

            StyledText {
                text: step.title
                font.pixelSize: Appearance.fontSize.normal
                font.weight: Font.DemiBold
            }
            StyledText {
                width: parent.width
                text: step.detail
                wrapMode: Text.Wrap
                color: Appearance.colors.onSurfaceVariant
                font.pixelSize: Appearance.fontSize.smallie
            }
        }
    }

    Stickers {
        width: root.width
        height: Math.max(root.height, column.y + column.height)
        key: "setup"
        count: 6
    }

    Column {
        id: column
        anchors.horizontalCenter: parent.horizontalCenter
        y: Math.max(36, (root.height - implicitHeight) / 2)
        width: Math.min(560, root.width - 72)
        spacing: 20

        MaterialSymbol {
            anchors.horizontalCenter: parent.horizontalCenter
            width: 64
            height: 64
            text: "library_music"
            iconSize: 56
            fill: 1
            color: Appearance.colors.primary
        }

        TitleText {
            anchors.horizontalCenter: parent.horizontalCenter
            text: qsTr("Connect Your Music")
            font.pixelSize: Appearance.fontSize.display
            font.weight: Font.Bold
        }

        SegmentedControl {
            visible: !root.chosen
            anchors.horizontalCenter: parent.horizontalCenter
            segmentHeight: 36
            currentIndex: root.youTube ? 1 : 0
            segments: [
                Segment { text: "Spotify" },
                Segment { text: "YouTube Music" }
            ]
            onActivated: (index) => root.youTube = index === 1
        }

        StyledText {
            visible: !root.chosen
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.Wrap
            text: root.youTube
                  ? qsTr("Search, play and keep your library from YouTube Music. Signing in is optional.")
                  : qsTr("Play your Spotify library. Needs Spotify Premium and a Client ID from your own developer app.")
            color: Appearance.colors.onSurfaceVariant
        }

        TextButton {
            visible: !root.chosen
            anchors.horizontalCenter: parent.horizontalCenter
            symbol: "arrow_forward"
            text: qsTr("Continue")
            onClicked: root.chosen = true
        }

        // youtube music: signing in is the account's own google sign-in, in a
        // window of the app; signed out it searches and plays all the same
        Column {
            width: parent.width
            spacing: 14
            visible: root.chosen && root.youTube

            StyledText {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.Wrap
                text: Session.youTubeBrowserLogin
                      ? qsTr("Sign in with your Google account to see your library, likes, playlists and history. The sign-in opens in a window of Swiftify and is kept in your keychain.")
                      : qsTr("Sign in to see your library, likes, playlists and history.")
                color: Appearance.colors.onSurfaceVariant
            }
            Column {
                id: cookieSteps
                // pasted cookies stand in where the sign-in window is missing or google turns it away
                property bool chosen: false
                visible: !Session.youTubeBrowserLogin || chosen
                width: parent.width
                spacing: 14

                Step {
                    number: 1
                    title: qsTr("Copy your YouTube Music cookies")
                    detail: qsTr("In a browser signed in to music.youtube.com, open the developer tools, pick any request under Network and copy its Cookie header. A cookies.txt export works too.")
                }
                TextButton {
                    x: 38
                    variant: TextButton.Text
                    symbol: "open_in_new"
                    text: qsTr("Open YouTube Music")
                    onClicked: App.openUrl("https://music.youtube.com")
                }
                Step {
                    number: 2
                    title: qsTr("Paste them here")
                    detail: qsTr("They are kept in your keychain, never in a file.")
                }
                OutlinedTextField {
                    id: youTubeCookies
                    anchors.horizontalCenter: parent.horizontalCenter
                    width: Math.min(460, parent.width)
                    placeholder: qsTr("Cookie header or cookies.txt from music.youtube.com")
                    onAccepted: if (signInButton.enabled) signInButton.clicked()
                }
            }
            Row {
                anchors.horizontalCenter: parent.horizontalCenter
                spacing: 12

                TextButton {
                    id: signInButton
                    symbol: "login"
                    text: qsTr("Sign In")
                    enabled: !Session.youTubeBusy
                             && (!cookieSteps.visible || youTubeCookies.text.trim().length > 0)
                    onClicked: {
                        Settings.musicService = "youtube"
                        if (cookieSteps.visible) {
                            Session.signInToYouTube(youTubeCookies.text)
                            youTubeCookies.text = ""
                        } else {
                            Session.beginYouTubeLogin()
                        }
                    }
                }
                TextButton {
                    variant: TextButton.Tonal
                    text: qsTr("Continue Without Signing In")
                    onClicked: Settings.musicService = "youtube"
                }
            }
            TextButton {
                visible: Session.youTubeBrowserLogin
                anchors.horizontalCenter: parent.horizontalCenter
                variant: TextButton.Text
                text: cookieSteps.chosen ? qsTr("Sign In With Google Instead") : qsTr("Paste Cookies Instead")
                onClicked: cookieSteps.chosen = !cookieSteps.chosen
            }
        }

        StyledText {
            visible: root.chosen && !root.youTube
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.Wrap
            text: qsTr("Swiftify needs a Client ID from your own Spotify developer app. You only have to set this up once.")
            color: Appearance.colors.onSurfaceVariant
        }

        Column {
            visible: root.chosen && !root.youTube
            width: parent.width
            spacing: 14

            Step {
                number: 1
                title: qsTr("Create a Spotify app")
                detail: qsTr("Open the Spotify Developer Dashboard, choose Create app, and select Web API if Spotify asks.")
            }

            TextButton {
                x: 38
                variant: TextButton.Text
                symbol: "open_in_new"
                text: qsTr("Open Spotify Developer Dashboard")
                onClicked: App.openUrl("https://developer.spotify.com/dashboard")
            }

            Step {
                number: 2
                title: qsTr("Add Swiftify's redirect URI")
                detail: qsTr("In the app settings, add this exact address under Redirect URIs, then save:")
            }

            Rectangle {
                x: 38
                width: redirect.implicitWidth + 24
                height: redirect.implicitHeight + 16
                radius: Appearance.rounding.verySmall
                color: Appearance.colors.surfaceContainerHigh

                TextEdit {
                    id: redirect
                    anchors.centerIn: parent
                    text: Session.redirectUri
                    readOnly: true
                    selectByMouse: true
                    font.family: Appearance.monoFontFamily
                    font.pixelSize: Appearance.fontSize.smallie
                    color: Appearance.colors.onSurface
                    selectionColor: Appearance.colors.primary
                    selectedTextColor: Appearance.colors.onPrimary
                }
            }

            Step {
                number: 3
                title: qsTr("Paste your Client ID")
                detail: qsTr("Copy the Client ID from the app settings. You do not need the Client Secret.")
            }
        }

        OutlinedTextField {
            visible: root.chosen && !root.youTube
            anchors.horizontalCenter: parent.horizontalCenter
            width: Math.min(460, parent.width)
            placeholder: qsTr("Spotify Client ID")
            text: Session.libraryClientId
            onTextEdited: Session.libraryClientId = text
            onAccepted: Session.beginLibraryLogin()
        }

        // the screen stays until both are connected (UiState.needsSetup):
        // each button says when its half is done, and the next one leads
        Row {
            visible: root.chosen && !root.youTube
            anchors.horizontalCenter: parent.horizontalCenter
            spacing: 12

            TextButton {
                variant: Session.libraryConnected ? TextButton.Tonal : TextButton.Filled
                symbol: Session.libraryConnected ? "check" : ""
                text: Session.libraryConnected ? qsTr("Library Connected") : qsTr("Connect Library")
                enabled: !Session.libraryConnected && Session.libraryClientId.trim().length > 0
                onClicked: Session.beginLibraryLogin()
            }
            TextButton {
                variant: Session.libraryConnected && !Session.connected ? TextButton.Filled : TextButton.Tonal
                symbol: Session.connected ? "check" : ""
                text: Session.connected ? qsTr("Playback Connected") : qsTr("Connect Playback")
                enabled: !Session.connected
                onClicked: Session.beginPlaybackLogin()
            }
        }

        StyledText {
            visible: root.chosen && !root.youTube
            anchors.horizontalCenter: parent.horizontalCenter
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.Wrap
            text: Session.libraryConnected && !Session.connected
                  ? qsTr("Your library is connected. Now connect playback so Swiftify can play music.")
                  : Session.connected && !Session.libraryConnected
                    ? qsTr("Playback is connected. Now connect your library to see your music.")
                    : qsTr("Connect Library first, then Connect Playback so Swiftify can play music.")
            font.pixelSize: Appearance.fontSize.smaller
            color: Session.libraryConnected !== Session.connected ? Appearance.colors.primary
                                                                  : Appearance.colors.onSurfaceVariant
        }

        TextButton {
            visible: root.chosen
            anchors.horizontalCenter: parent.horizontalCenter
            variant: TextButton.Text
            symbol: "arrow_back"
            text: qsTr("Choose Another Service")
            onClicked: root.chosen = false
        }
    }
}
