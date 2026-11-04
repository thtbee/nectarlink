// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick

// Where a window of this size goes when it asks for a spot: there, on any
// monitor, including those left of or above the main one (negative coordinates),
// as long as enough of its title bar stays reachable. Otherwise, the nearest spot
// on `fallback`.
QtObject {
    function spot(px, py, w, h, screens, fallback) {
        for (let i = 0; i < screens.length; i++) {
            const s = screens[i]
            if (px + w > s.virtualX + 80 && px < s.virtualX + s.width - 80
                    && py >= s.virtualY && py < s.virtualY + s.height - 80)
                return { x: px, y: py }
        }
        return {
            x: Math.max(fallback.virtualX, Math.min(fallback.virtualX + fallback.width - w, px)),
            y: Math.max(fallback.virtualY, Math.min(fallback.virtualY + fallback.height - h, py))
        }
    }
}
