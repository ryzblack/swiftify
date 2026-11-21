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

#include <QUrl>

namespace swiftify {

/// opens a web address in the person's browser. inside a flatpak it asks the
/// desktop portal directly: Qt reaches the portal only through its portal
/// theme, which a QT_QPA_PLATFORMTHEME set outside the sandbox (qt6ct, say)
/// replaces, and the link then opens nowhere. falls back to QDesktopServices
void openExternally(const QUrl &url);

} // namespace swiftify
