// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import android.content.pm.PackageManager

/**
 * Root managers that show `su` only to apps on their Superuser list, so before
 * NectarLink is allowed there, finding the manager is the only sign of root.
 * Magisk shows `su` to every app, so it isn't needed here.
 */
object RootManagers {
    val PACKAGES = listOf(
        "me.weishu.kernelsu", // KernelSU
        "com.rifsxd.ksunext", // KernelSU Next
        "com.sukisu.ultra", // SukiSU Ultra
        "me.bmax.apatch", // APatch
    )

    fun anyInstalled(packages: PackageManager): Boolean = PACKAGES.any { name ->
        runCatching {
            packages.getPackageInfo(name, 0)
            true
        }.getOrDefault(false)
    }
}
