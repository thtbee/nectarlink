// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.core

import android.annotation.SuppressLint
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.content.pm.PackageManager
import android.os.Build
import android.os.PowerManager
import android.provider.Settings
import androidx.core.net.toUri
import app.nectarlink.android.BuildConfig
import app.nectarlink.android.R
import java.util.Locale

/**
 * The local network: Android 17 lets an app reach devices on it (the PC)
 * only with the user's permission.
 */
object LocalNetwork {
    /** Android 17's permission (API 37). */
    const val PERMISSION = "android.permission.ACCESS_LOCAL_NETWORK"

    fun needed(): Boolean = Build.VERSION.SDK_INT >= 37

    fun granted(context: Context): Boolean =
        !needed() || context.checkSelfPermission(PERMISSION) == PackageManager.PERMISSION_GRANTED
}

enum class OemBrand {
    SAMSUNG,
    XIAOMI,
    ONEPLUS,
    OPPO_REALME,
    VIVO,
    HUAWEI,
    PIXEL,
    OTHER,
}

/**
 * Running unrestricted in the background. Without it, battery savers (and
 * OEM background app killers in particular) pause NectarLink between events,
 * so a PC's requests (replies, dismissals, find my phone) time out.
 */
object BackgroundAccess {
    fun isUnrestricted(context: Context): Boolean =
        context.getSystemService(PowerManager::class.java)?.isIgnoringBatteryOptimizations(context.packageName) ?: true

    /**
     * Android's prompt to let NectarLink run unrestricted. The Play build can't
     * ask (Play keeps that for apps whose core function needs it), so it opens
     * the list of apps' battery settings, where the user picks NectarLink.
     */
    @SuppressLint("BatteryLife") // A companion that must stay reachable is the case this exists for.
    fun requestIntent(context: Context): Intent =
        if (BuildConfig.PLAY) {
            Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        } else {
            Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS, "package:${context.packageName}".toUri())
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        }

    /** Detects the phone's OEM family from `Build.MANUFACTURER` and `Build.BRAND`. */
    fun detectOem(
        manufacturer: String = Build.MANUFACTURER.orEmpty(),
        brand: String = Build.BRAND.orEmpty(),
    ): OemBrand {
        val combined = "$manufacturer $brand".lowercase(Locale.ROOT)
        return when {
            "samsung" in combined -> OemBrand.SAMSUNG
            "xiaomi" in combined || "redmi" in combined || "poco" in combined -> OemBrand.XIAOMI
            "oneplus" in combined -> OemBrand.ONEPLUS
            "oppo" in combined || "realme" in combined -> OemBrand.OPPO_REALME
            "vivo" in combined || "iqoo" in combined -> OemBrand.VIVO
            "huawei" in combined || "honor" in combined -> OemBrand.HUAWEI
            "google" in combined || "pixel" in combined -> OemBrand.PIXEL
            else -> OemBrand.OTHER
        }
    }

    /**
     * Step-by-step battery guide string resource for **only** the user's phone
     * manufacturer, or `null` on unlisted OEMs.
     */
    fun oemGuideResId(oem: OemBrand = detectOem()): Int? = when (oem) {
        OemBrand.SAMSUNG -> R.string.battery_guide_samsung
        OemBrand.XIAOMI -> R.string.battery_guide_xiaomi
        OemBrand.ONEPLUS -> R.string.battery_guide_oneplus
        OemBrand.OPPO_REALME -> R.string.battery_guide_oppo_realme
        OemBrand.VIVO -> R.string.battery_guide_vivo
        OemBrand.HUAWEI -> R.string.battery_guide_huawei
        OemBrand.PIXEL -> R.string.battery_guide_pixel
        OemBrand.OTHER -> null
    }

    /** Candidate OEM-specific battery / autostart settings intents for [oem]. */
    fun candidateOemIntents(packageName: String, oem: OemBrand): List<Intent> = when (oem) {
        OemBrand.SAMSUNG -> listOf(
            Intent().setComponent(
                ComponentName("com.samsung.android.lool", "com.samsung.android.sm.battery.ui.BatteryActivity"),
            ),
            Intent().setComponent(
                ComponentName("com.samsung.android.sm", "com.samsung.android.sm.ui.battery.BatteryActivity"),
            ),
        )
        OemBrand.XIAOMI -> listOf(
            Intent().setComponent(
                ComponentName(
                    "com.miui.securitycenter",
                    "com.miui.permcenter.autostart.AutoStartManagementActivity",
                ),
            ),
            Intent().setComponent(
                ComponentName("com.miui.powerkeeper", "com.miui.powerkeeper.ui.HiddenAppsConfigActivity"),
            ).putExtra("package_name", packageName),
        )
        OemBrand.ONEPLUS -> listOf(
            Intent().setComponent(
                ComponentName(
                    "com.oneplus.security",
                    "com.oneplus.security.chainlaunch.view.ChainLaunchAppListActivity",
                ),
            ),
            Intent().setComponent(
                ComponentName("com.oplus.battery", "com.oplus.powermanager.fuelgaue.PowerControlActivity"),
            ),
        )
        OemBrand.OPPO_REALME -> listOf(
            Intent().setComponent(
                ComponentName(
                    "com.coloros.safecenter",
                    "com.coloros.safecenter.permission.startup.StartupAppListActivity",
                ),
            ),
            Intent().setComponent(
                ComponentName("com.oplus.battery", "com.oplus.powermanager.fuelgaue.PowerControlActivity"),
            ),
            Intent().setComponent(
                ComponentName(
                    "com.coloros.oppoguardelf",
                    "com.coloros.powermanager.fuelgaue.PowerUsageModelActivity",
                ),
            ),
        )
        OemBrand.VIVO -> listOf(
            Intent().setComponent(
                ComponentName(
                    "com.vivo.permissionmanager",
                    "com.vivo.permissionmanager.activity.BgStartUpManagerActivity",
                ),
            ),
            Intent().setComponent(
                ComponentName("com.iqoo.secure", "com.iqoo.secure.ui.phoneoptimize.BgStartUpManager"),
            ),
        )
        OemBrand.HUAWEI -> listOf(
            Intent().setComponent(
                ComponentName(
                    "com.huawei.systemmanager",
                    "com.huawei.systemmanager.startupmgr.ui.StartupNormalAppListActivity",
                ),
            ),
            Intent().setComponent(
                ComponentName(
                    "com.huawei.systemmanager",
                    "com.huawei.systemmanager.optimize.process.ProtectActivity",
                ),
            ),
        )
        OemBrand.PIXEL -> listOf(
            Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, "package:$packageName".toUri()),
        )
        OemBrand.OTHER -> emptyList()
    }

    /**
     * Returns the first OEM battery/autostart settings intent that resolves on
     * this phone, or falls back to NectarLink's App info screen (`ACTION_APPLICATION_DETAILS_SETTINGS`).
     */
    fun oemSettingsIntent(context: Context, oem: OemBrand = detectOem()): Intent {
        val pm = context.packageManager
        for (intent in candidateOemIntents(context.packageName, oem)) {
            val resolves = runCatching {
                pm.resolveActivity(intent, PackageManager.MATCH_DEFAULT_ONLY) != null
            }.getOrDefault(false)
            if (resolves) {
                return intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            }
        }
        return appDetailsIntent(context)
    }

    fun appDetailsIntent(context: Context): Intent =
        Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, "package:${context.packageName}".toUri())
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)

    /**
     * Whether NectarLink was installed other than by an app store (from a
     * file, a browser download, or another installer). Android 13 and newer
     * may then hold back some settings ("restricted settings") until the user
     * allows them from App info. There is no public way to read that state
     * itself, so callers show help only after a grant didn't go through.
     */
    fun isSideloaded(context: Context): Boolean {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return false
        val source = runCatching {
            context.packageManager.getInstallSourceInfo(context.packageName).packageSource
        }.getOrNull() ?: return true
        return source != PackageInstaller.PACKAGE_SOURCE_STORE
    }
}

