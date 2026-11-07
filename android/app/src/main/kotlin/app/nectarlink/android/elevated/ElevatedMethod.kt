// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

/** The ways Elevated's helper can be started. */
enum class ElevatedMethod {
    WirelessDebugging,
    Shizuku,
    Root,
    ;

    companion object {
        /** What this phone can use. Wireless debugging is always there. */
        fun available(shizukuInstalled: Boolean, rooted: Boolean): List<ElevatedMethod> = buildList {
            add(WirelessDebugging)
            if (shizukuInstalled) add(Shizuku)
            if (rooted) add(Root)
        }
    }
}
