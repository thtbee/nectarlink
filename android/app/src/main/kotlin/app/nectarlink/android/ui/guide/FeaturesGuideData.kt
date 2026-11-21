// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.guide

import android.content.Context
import androidx.annotation.StringRes
import app.nectarlink.android.R
import app.nectarlink.android.core.CoreState

/**
 * High-level area filter tabs on the Features Guide screen.
 */
enum class FeatureArea(
    val id: String,
    @StringRes val labelRes: Int,
) {
    All("all", R.string.guide_area_all),
    NotificationsCalls("notifications_calls", R.string.guide_area_notifications_calls),
    Messages("messages", R.string.guide_area_messages),
    ClipboardFiles("clipboard_files", R.string.guide_area_clipboard_files),
    MirroringCamera("mirroring_camera", R.string.guide_area_mirroring_camera),
    InputControl("input_control", R.string.guide_area_input_control),
    PowerNetwork("power_network", R.string.guide_area_power_network),
}

/**
 * In-app destinations that "Show me" can navigate to, scroll into view, and pulse-highlight.
 */
enum class GuideHighlightTarget {
    HOME_PAIR_PC,
    HOME_PC_CARD,
    HOME_PC_AUDIO,
    HOME_CLIPBOARD_HISTORY,
    HOME_TIMELINE,
    HOME_SEND_QUEUE,
    HOME_SETUP_PERMISSIONS,
    HOME_LOCALSEND,
    SETTINGS_DEVICES,
    SETTINGS_NOTIFICATIONS,
    SETTINGS_CONTROL,
    SETTINGS_BACKGROUND,
    SETTINGS_CALLS_CONTACTS,
    SETTINGS_PHOTOS_STORAGE,
    SETTINGS_CLIPBOARD,
    SETTINGS_LOCALSEND,
    SETTINGS_DATA_STORAGE,
}

/**
 * Quick action to grant a missing permission or enable a missing capability directly from a feature card.
 */
enum class GuideFixAction(
    @StringRes val buttonLabelRes: Int,
) {
    GRANT_NOTIFICATIONS(R.string.guide_fix_grant),
    GRANT_BATTERY(R.string.guide_fix_allow),
    GRANT_CALLS(R.string.guide_fix_grant),
    GRANT_SMS_CONTACTS(R.string.guide_fix_grant),
    GRANT_PHOTOS(R.string.guide_fix_grant),
    GRANT_STORAGE(R.string.guide_fix_grant),
    OPEN_ACCESSIBILITY(R.string.guide_fix_enable),
    ENABLE_ELEVATED(R.string.guide_fix_enable),
    ENABLE_DND(R.string.guide_fix_grant),
    ENABLE_WRITE_SETTINGS(R.string.guide_fix_grant),
    PAIR_PC(R.string.guide_fix_pair),
}

/**
 * Live availability badge shown on each feature card.
 */
sealed interface FeatureLiveBadge {
    @get:StringRes
    val labelRes: Int

    data object Available : FeatureLiveBadge {
        override val labelRes: Int = R.string.guide_badge_available
    }

    data class NeedsPermission(
        @StringRes val permissionNameRes: Int,
    ) : FeatureLiveBadge {
        override val labelRes: Int = R.string.guide_badge_needs_permission
    }

    data class NeedsPowerLevel(
        @StringRes val levelNameRes: Int,
    ) : FeatureLiveBadge {
        override val labelRes: Int = R.string.guide_badge_needs_power
    }

    data object ConnectPcToCheck : FeatureLiveBadge {
        override val labelRes: Int = R.string.guide_badge_connect_pc
    }
}

/**
 * Complete documentation and live-status metadata for one feature in the Features Guide.
 */
@androidx.compose.runtime.Immutable
data class FeatureGuideEntry(
    val id: String,
    val area: FeatureArea,
    @StringRes val titleRes: Int,
    @StringRes val summaryRes: Int,
    val keywords: List<String>,
    @StringRes val whereTextRes: Int,
    val showMeTarget: GuideHighlightTarget?,
    val externalStepResIds: List<Int> = emptyList(),
    @StringRes val needsSummaryRes: Int,
    val whatYouNeedResIds: List<Int>,
    val stepByStepPhoneResIds: List<Int>,
    val stepByStepPcResIds: List<Int>,
    val optionsExplainedResIds: List<Int>,
    @StringRes val dataAndPrivacyRes: Int,
    @StringRes val limitsRes: Int,
    val evaluateBadge: (CoreState) -> FeatureLiveBadge,
    val evaluateFixAction: (CoreState) -> GuideFixAction?,
)

/**
 * Full catalog of NectarLink features for the Android Features Guide.
 */
object FeaturesGuideCatalog {
    val entries: List<FeatureGuideEntry> = listOf(
        // 1. Notifications
        FeatureGuideEntry(
            id = "notification_mirroring",
            area = FeatureArea.NotificationsCalls,
            titleRes = R.string.guide_notif_title,
            summaryRes = R.string.guide_notif_summary,
            keywords = listOf("notifications", "alerts", "reply", "dismiss", "mirror", "toast"),
            whereTextRes = R.string.guide_notif_where,
            showMeTarget = GuideHighlightTarget.SETTINGS_NOTIFICATIONS,
            needsSummaryRes = R.string.guide_notif_needs,
            whatYouNeedResIds = listOf(
                R.string.guide_notif_need_1,
                R.string.guide_notif_need_2,
                R.string.guide_notif_need_3,
            ),
            stepByStepPhoneResIds = listOf(
                R.string.guide_notif_phone_1,
                R.string.guide_notif_phone_2,
            ),
            stepByStepPcResIds = listOf(
                R.string.guide_notif_pc_1,
                R.string.guide_notif_pc_2,
            ),
            optionsExplainedResIds = listOf(
                R.string.guide_notif_opt_1,
                R.string.guide_notif_opt_2,
            ),
            dataAndPrivacyRes = R.string.guide_notif_privacy,
            limitsRes = R.string.guide_notif_limits,
            evaluateBadge = { state ->
                if (!state.notificationAccess) {
                    FeatureLiveBadge.NeedsPermission(R.string.guide_req_notification_access)
                } else if (state.devices.none { it.online }) {
                    FeatureLiveBadge.ConnectPcToCheck
                } else {
                    FeatureLiveBadge.Available
                }
            },
            evaluateFixAction = { state ->
                if (!state.notificationAccess) GuideFixAction.GRANT_NOTIFICATIONS else null
            },
        ),

        // 2. Calls
        FeatureGuideEntry(
            id = "incoming_calls",
            area = FeatureArea.NotificationsCalls,
            titleRes = R.string.guide_calls_title,
            summaryRes = R.string.guide_calls_summary,
            keywords = listOf("calls", "phone", "dial", "ring", "hang up", "answer", "call log", "speakerphone"),
            whereTextRes = R.string.guide_calls_where,
            showMeTarget = GuideHighlightTarget.SETTINGS_CALLS_CONTACTS,
            needsSummaryRes = R.string.guide_calls_needs,
            whatYouNeedResIds = listOf(
                R.string.guide_calls_need_1,
                R.string.guide_calls_need_2,
            ),
            stepByStepPhoneResIds = listOf(
                R.string.guide_calls_phone_1,
                R.string.guide_calls_phone_2,
            ),
            stepByStepPcResIds = listOf(
                R.string.guide_calls_pc_1,
                R.string.guide_calls_pc_2,
            ),
            optionsExplainedResIds = listOf(
                R.string.guide_calls_opt_1,
                R.string.guide_calls_opt_2,
            ),
            dataAndPrivacyRes = R.string.guide_calls_privacy,
            limitsRes = R.string.guide_calls_limits,
            evaluateBadge = { state ->
                if (!state.callAccess) {
                    FeatureLiveBadge.NeedsPermission(R.string.guide_req_calls)
                } else if (state.devices.none { it.online }) {
                    FeatureLiveBadge.ConnectPcToCheck
                } else {
                    FeatureLiveBadge.Available
                }
            },
            evaluateFixAction = { state ->
                if (!state.callAccess) GuideFixAction.GRANT_CALLS else null
            },
        ),

        // 3. Now playing
        FeatureGuideEntry(
            id = "media_and_pc_audio",
            area = FeatureArea.NotificationsCalls,
            titleRes = R.string.guide_media_title,
            summaryRes = R.string.guide_media_summary,
            keywords = listOf("now playing", "media", "music", "audio", "speaker", "volume", "mute", "spotify", "youtube"),
            whereTextRes = R.string.guide_media_where,
            showMeTarget = GuideHighlightTarget.HOME_PC_CARD,
            needsSummaryRes = R.string.guide_media_needs,
            whatYouNeedResIds = listOf(
                R.string.guide_media_need_1,
                R.string.guide_media_need_2,
            ),
            stepByStepPhoneResIds = listOf(
                R.string.guide_media_phone_1,
                R.string.guide_media_phone_2,
            ),
            stepByStepPcResIds = listOf(
                R.string.guide_media_pc_1,
            ),
            optionsExplainedResIds = listOf(
                R.string.guide_media_opt_1,
                R.string.guide_media_opt_2,
            ),
            dataAndPrivacyRes = R.string.guide_media_privacy,
            limitsRes = R.string.guide_media_limits,
            evaluateBadge = { state ->
                if (!state.notificationAccess) {
                    FeatureLiveBadge.NeedsPermission(R.string.guide_req_notification_access)
                } else if (state.devices.none { it.online }) {
                    FeatureLiveBadge.ConnectPcToCheck
                } else {
                    FeatureLiveBadge.Available
                }
            },
            evaluateFixAction = { state ->
                if (!state.notificationAccess) GuideFixAction.GRANT_NOTIFICATIONS else null
            },
        ),

        // 4. Messages
        FeatureGuideEntry(
            id = "sms_mms_messages",
            area = FeatureArea.Messages,
            titleRes = R.string.guide_sms_title,
            summaryRes = R.string.guide_sms_summary,
            keywords = listOf("messages", "sms", "mms", "text", "chat", "contacts", "attachments"),
            whereTextRes = R.string.guide_sms_where,
            showMeTarget = GuideHighlightTarget.SETTINGS_CALLS_CONTACTS,
            needsSummaryRes = R.string.guide_sms_needs,
            whatYouNeedResIds = listOf(
                R.string.guide_sms_need_1,
                R.string.guide_sms_need_2,
            ),
            stepByStepPhoneResIds = listOf(
                R.string.guide_sms_phone_1,
            ),
            stepByStepPcResIds = listOf(
                R.string.guide_sms_pc_1,
            ),
            optionsExplainedResIds = listOf(
                R.string.guide_sms_opt_1,
                R.string.guide_sms_opt_2,
            ),
            dataAndPrivacyRes = R.string.guide_sms_privacy,
            limitsRes = R.string.guide_sms_limits,
            evaluateBadge = { state ->
                if (!state.smsAccess || !state.contactsAccess) {
                    FeatureLiveBadge.NeedsPermission(R.string.guide_req_sms_contacts)
                } else if (state.devices.none { it.online }) {
                    FeatureLiveBadge.ConnectPcToCheck
                } else {
                    FeatureLiveBadge.Available
                }
            },
            evaluateFixAction = { state ->
                if (!state.smsAccess || !state.contactsAccess) GuideFixAction.GRANT_SMS_CONTACTS else null
            },
        ),

        // 5. Clipboard
        FeatureGuideEntry(
            id = "clipboard_sync",
            area = FeatureArea.ClipboardFiles,
            titleRes = R.string.guide_clip_title,
            summaryRes = R.string.guide_clip_summary,
            keywords = listOf("clipboard", "copy", "paste", "history", "clips", "text", "image"),
            whereTextRes = R.string.guide_clip_where,
            showMeTarget = GuideHighlightTarget.HOME_CLIPBOARD_HISTORY,
            needsSummaryRes = R.string.guide_clip_needs,
            whatYouNeedResIds = listOf(
                R.string.guide_clip_need_1,
                R.string.guide_clip_need_2,
            ),
            stepByStepPhoneResIds = listOf(
                R.string.guide_clip_phone_1,
                R.string.guide_clip_phone_2,
            ),
            stepByStepPcResIds = listOf(
                R.string.guide_clip_pc_1,
            ),
            optionsExplainedResIds = listOf(
                R.string.guide_clip_opt_1,
                R.string.guide_clip_opt_2,
                R.string.guide_clip_opt_3,
            ),
            dataAndPrivacyRes = R.string.guide_clip_privacy,
            limitsRes = R.string.guide_clip_limits,
            evaluateBadge = { state ->
                if (state.devices.none { it.online }) {
                    FeatureLiveBadge.ConnectPcToCheck
                } else {
                    FeatureLiveBadge.Available
                }
            },
            evaluateFixAction = { null },
        ),

        // 6. Send clipboard
        FeatureGuideEntry(
            id = "quick_settings_clipboard_tile",
            area = FeatureArea.ClipboardFiles,
            titleRes = R.string.guide_tile_title,
            summaryRes = R.string.guide_tile_summary,
            keywords = listOf("send clipboard", "quick settings", "tile", "clipboard", "shade"),
            whereTextRes = R.string.guide_tile_where,
            showMeTarget = null,
            externalStepResIds = listOf(
                R.string.guide_tile_ext_1,
                R.string.guide_tile_ext_2,
                R.string.guide_tile_ext_3,
            ),
            needsSummaryRes = R.string.guide_tile_needs,
            whatYouNeedResIds = listOf(
                R.string.guide_tile_need_1,
            ),
            stepByStepPhoneResIds = listOf(
                R.string.guide_tile_phone_1,
            ),
            stepByStepPcResIds = listOf(
                R.string.guide_tile_pc_1,
            ),
            optionsExplainedResIds = listOf(
                R.string.guide_tile_opt_1,
            ),
            dataAndPrivacyRes = R.string.guide_tile_privacy,
            limitsRes = R.string.guide_tile_limits,
            evaluateBadge = { state ->
                if (state.devices.none { it.online }) {
                    FeatureLiveBadge.ConnectPcToCheck
                } else {
                    FeatureLiveBadge.Available
                }
            },
            evaluateFixAction = { null },
        ),

        // 7. Send files
        FeatureGuideEntry(
            id = "send_files_and_folders",
            area = FeatureArea.ClipboardFiles,
            titleRes = R.string.guide_files_title,
            summaryRes = R.string.guide_files_summary,
            keywords = listOf("send files", "files", "folders", "transfer", "send", "receive", "downloads", "usb", "wifi"),
            whereTextRes = R.string.guide_files_where,
            showMeTarget = GuideHighlightTarget.HOME_PC_CARD,
            needsSummaryRes = R.string.guide_files_needs,
            whatYouNeedResIds = listOf(
                R.string.guide_files_need_1,
            ),
            stepByStepPhoneResIds = listOf(
                R.string.guide_files_phone_1,
                R.string.guide_files_phone_2,
            ),
            stepByStepPcResIds = listOf(
                R.string.guide_files_pc_1,
            ),
            optionsExplainedResIds = listOf(
                R.string.guide_files_opt_1,
                R.string.guide_files_opt_2,
            ),
            dataAndPrivacyRes = R.string.guide_files_privacy,
            limitsRes = R.string.guide_files_limits,
            evaluateBadge = { state ->
                if (state.devices.none { it.online }) {
                    FeatureLiveBadge.ConnectPcToCheck
                } else {
                    FeatureLiveBadge.Available
                }
            },
            evaluateFixAction = { null },
        ),

        // 8. Send queue
        FeatureGuideEntry(
            id = "waiting_to_send_queue",
            area = FeatureArea.ClipboardFiles,
            titleRes = R.string.guide_queue_title,
            summaryRes = R.string.guide_queue_summary,
            keywords = listOf("send queue", "queue", "waiting to send", "offline", "retry", "cancel", "stage"),
            whereTextRes = R.string.guide_queue_where,
            showMeTarget = GuideHighlightTarget.HOME_SEND_QUEUE,
            needsSummaryRes = R.string.guide_queue_needs,
            whatYouNeedResIds = listOf(
                R.string.guide_queue_need_1,
            ),
            stepByStepPhoneResIds = listOf(
                R.string.guide_queue_phone_1,
                R.string.guide_queue_phone_2,
            ),
            stepByStepPcResIds = listOf(
                R.string.guide_queue_pc_1,
            ),
            optionsExplainedResIds = listOf(
                R.string.guide_queue_opt_1,
            ),
            dataAndPrivacyRes = R.string.guide_queue_privacy,
            limitsRes = R.string.guide_queue_limits,
            evaluateBadge = { state ->
                if (state.devices.none { it.online }) {
                    FeatureLiveBadge.ConnectPcToCheck
                } else {
                    FeatureLiveBadge.Available
                }
            },
            evaluateFixAction = { null },
        ),

        // 9. Send to PC
        FeatureGuideEntry(
            id = "share_sheet_handoff",
            area = FeatureArea.ClipboardFiles,
            titleRes = R.string.guide_share_title,
            summaryRes = R.string.guide_share_summary,
            keywords = listOf("send to pc", "share", "handoff", "open on pc", "link", "url", "browser", "document"),
            whereTextRes = R.string.guide_share_where,
            showMeTarget = null,
            externalStepResIds = listOf(
                R.string.guide_share_ext_1,
                R.string.guide_share_ext_2,
            ),
            needsSummaryRes = R.string.guide_share_needs,
            whatYouNeedResIds = listOf(
                R.string.guide_share_need_1,
            ),
            stepByStepPhoneResIds = listOf(
                R.string.guide_share_phone_1,
            ),
            stepByStepPcResIds = listOf(
                R.string.guide_share_pc_1,
            ),
            optionsExplainedResIds = listOf(
                R.string.guide_share_opt_1,
            ),
            dataAndPrivacyRes = R.string.guide_share_privacy,
            limitsRes = R.string.guide_share_limits,
            evaluateBadge = { state ->
                if (state.devices.none { it.online }) {
                    FeatureLiveBadge.ConnectPcToCheck
                } else {
                    FeatureLiveBadge.Available
                }
            },
            evaluateFixAction = { null },
        ),

        // 10. Storage in File Explorer
        FeatureGuideEntry(
            id = "browse_phone_storage",
            area = FeatureArea.ClipboardFiles,
            titleRes = R.string.guide_storage_title,
            summaryRes = R.string.guide_storage_summary,
            keywords = listOf("storage in file explorer", "storage", "file explorer", "mount", "folders", "saf", "browse"),
            whereTextRes = R.string.guide_storage_where,
            showMeTarget = GuideHighlightTarget.SETTINGS_PHOTOS_STORAGE,
            needsSummaryRes = R.string.guide_storage_needs,
            whatYouNeedResIds = listOf(
                R.string.guide_storage_need_1,
                R.string.guide_storage_need_2,
            ),
            stepByStepPhoneResIds = listOf(
                R.string.guide_storage_phone_1,
                R.string.guide_storage_phone_2,
            ),
            stepByStepPcResIds = listOf(
                R.string.guide_storage_pc_1,
            ),
            optionsExplainedResIds = listOf(
                R.string.guide_storage_opt_1,
            ),
            dataAndPrivacyRes = R.string.guide_storage_privacy,
            limitsRes = R.string.guide_storage_limits,
            evaluateBadge = { state ->
                val hasStoragePerm = state.storageAllFilesAccess || state.storageSafFolders.isNotEmpty()
                if (!hasStoragePerm) {
                    FeatureLiveBadge.NeedsPermission(R.string.guide_req_storage)
                } else if (state.devices.none { it.online }) {
                    FeatureLiveBadge.ConnectPcToCheck
                } else {
                    FeatureLiveBadge.Available
                }
            },
            evaluateFixAction = { state ->
                val hasStoragePerm = state.storageAllFilesAccess || state.storageSafFolders.isNotEmpty()
                if (!hasStoragePerm) GuideFixAction.GRANT_STORAGE else null
            },
        ),

        // 11. Browse PC files
        FeatureGuideEntry(
            id = "browse_pc_files",
            area = FeatureArea.ClipboardFiles,
            titleRes = R.string.guide_pc_files_title,
            summaryRes = R.string.guide_pc_files_summary,
            keywords = listOf("browse pc files", "browse pc", "pc files", "download", "remote storage", "folders"),
            whereTextRes = R.string.guide_pc_files_where,
            showMeTarget = GuideHighlightTarget.HOME_PC_CARD,
            needsSummaryRes = R.string.guide_pc_files_needs,
            whatYouNeedResIds = listOf(
                R.string.guide_pc_files_need_1,
            ),
            stepByStepPhoneResIds = listOf(
                R.string.guide_pc_files_phone_1,
            ),
            stepByStepPcResIds = listOf(
                R.string.guide_pc_files_pc_1,
            ),
            optionsExplainedResIds = listOf(
                R.string.guide_pc_files_opt_1,
            ),
            dataAndPrivacyRes = R.string.guide_pc_files_privacy,
            limitsRes = R.string.guide_pc_files_limits,
            evaluateBadge = { state ->
                if (state.devices.none { it.online }) {
                    FeatureLiveBadge.ConnectPcToCheck
                } else {
                    FeatureLiveBadge.Available
                }
            },
            evaluateFixAction = { null },
        ),

        // 12. Photos
        FeatureGuideEntry(
            id = "recent_photos",
            area = FeatureArea.ClipboardFiles,
            titleRes = R.string.guide_photos_title,
            summaryRes = R.string.guide_photos_summary,
            keywords = listOf("photos", "gallery", "camera roll", "pictures", "images", "thumbnails"),
            whereTextRes = R.string.guide_photos_where,
            showMeTarget = GuideHighlightTarget.SETTINGS_PHOTOS_STORAGE,
            needsSummaryRes = R.string.guide_photos_needs,
            whatYouNeedResIds = listOf(
                R.string.guide_photos_need_1,
            ),
            stepByStepPhoneResIds = listOf(
                R.string.guide_photos_phone_1,
            ),
            stepByStepPcResIds = listOf(
                R.string.guide_photos_pc_1,
            ),
            optionsExplainedResIds = listOf(
                R.string.guide_photos_opt_1,
            ),
            dataAndPrivacyRes = R.string.guide_photos_privacy,
            limitsRes = R.string.guide_photos_limits,
            evaluateBadge = { state ->
                if (!state.photoAccess) {
                    FeatureLiveBadge.NeedsPermission(R.string.guide_req_photos)
                } else if (state.devices.none { it.online }) {
                    FeatureLiveBadge.ConnectPcToCheck
                } else {
                    FeatureLiveBadge.Available
                }
            },
            evaluateFixAction = { state ->
                if (!state.photoAccess) GuideFixAction.GRANT_PHOTOS else null
            },
        ),

        // 13. Timeline
        FeatureGuideEntry(
            id = "timeline_history",
            area = FeatureArea.ClipboardFiles,
            titleRes = R.string.guide_timeline_title,
            summaryRes = R.string.guide_timeline_summary,
            keywords = listOf("timeline", "history", "activity", "log", "search", "resend"),
            whereTextRes = R.string.guide_timeline_where,
            showMeTarget = GuideHighlightTarget.HOME_TIMELINE,
            needsSummaryRes = R.string.guide_timeline_needs,
            whatYouNeedResIds = listOf(
                R.string.guide_timeline_need_1,
            ),
            stepByStepPhoneResIds = listOf(
                R.string.guide_timeline_phone_1,
            ),
            stepByStepPcResIds = listOf(
                R.string.guide_timeline_pc_1,
            ),
            optionsExplainedResIds = listOf(
                R.string.guide_timeline_opt_1,
            ),
            dataAndPrivacyRes = R.string.guide_timeline_privacy,
            limitsRes = R.string.guide_timeline_limits,
            evaluateBadge = { FeatureLiveBadge.Available },
            evaluateFixAction = { null },
        ),

        // 14. LocalSend
        FeatureGuideEntry(
            id = "localsend_lan",
            area = FeatureArea.ClipboardFiles,
            titleRes = R.string.guide_localsend_title,
            summaryRes = R.string.guide_localsend_summary,
            keywords = listOf("localsend", "lan", "nearby", "unpaired", "share", "wifi"),
            whereTextRes = R.string.guide_localsend_where,
            showMeTarget = GuideHighlightTarget.SETTINGS_LOCALSEND,
            needsSummaryRes = R.string.guide_localsend_needs,
            whatYouNeedResIds = listOf(
                R.string.guide_localsend_need_1,
            ),
            stepByStepPhoneResIds = listOf(
                R.string.guide_localsend_phone_1,
            ),
            stepByStepPcResIds = listOf(
                R.string.guide_localsend_pc_1,
            ),
            optionsExplainedResIds = listOf(
                R.string.guide_localsend_opt_1,
            ),
            dataAndPrivacyRes = R.string.guide_localsend_privacy,
            limitsRes = R.string.guide_localsend_limits,
            evaluateBadge = { FeatureLiveBadge.Available },
            evaluateFixAction = { null },
        ),

        // 15. Screen mirroring
        FeatureGuideEntry(
            id = "phone_screen_mirroring",
            area = FeatureArea.MirroringCamera,
            titleRes = R.string.guide_mirror_title,
            summaryRes = R.string.guide_mirror_summary,
            keywords = listOf("screen mirroring", "mirror", "screen", "cast", "display", "audio", "stream", "view pc screen"),
            whereTextRes = R.string.guide_mirror_where,
            showMeTarget = GuideHighlightTarget.SETTINGS_CONTROL,
            needsSummaryRes = R.string.guide_mirror_needs,
            whatYouNeedResIds = listOf(
                R.string.guide_mirror_need_1,
                R.string.guide_mirror_need_2,
            ),
            stepByStepPhoneResIds = listOf(
                R.string.guide_mirror_phone_1,
                R.string.guide_mirror_phone_2,
            ),
            stepByStepPcResIds = listOf(
                R.string.guide_mirror_pc_1,
            ),
            optionsExplainedResIds = listOf(
                R.string.guide_mirror_opt_1,
            ),
            dataAndPrivacyRes = R.string.guide_mirror_privacy,
            limitsRes = R.string.guide_mirror_limits,
            evaluateBadge = { state ->
                if (state.devices.none { it.online }) {
                    FeatureLiveBadge.ConnectPcToCheck
                } else {
                    FeatureLiveBadge.Available
                }
            },
            evaluateFixAction = { null },
        ),

        // 16. App streams
        FeatureGuideEntry(
            id = "phone_app_windows",
            area = FeatureArea.MirroringCamera,
            titleRes = R.string.guide_apps_title,
            summaryRes = R.string.guide_apps_summary,
            keywords = listOf("app streams", "app window", "virtual display", "elevated", "wireless debugging", "shizuku", "apps"),
            whereTextRes = R.string.guide_apps_where,
            showMeTarget = GuideHighlightTarget.SETTINGS_CONTROL,
            needsSummaryRes = R.string.guide_apps_needs,
            whatYouNeedResIds = listOf(
                R.string.guide_apps_need_1,
                R.string.guide_apps_need_2,
            ),
            stepByStepPhoneResIds = listOf(
                R.string.guide_apps_phone_1,
            ),
            stepByStepPcResIds = listOf(
                R.string.guide_apps_pc_1,
            ),
            optionsExplainedResIds = listOf(
                R.string.guide_apps_opt_1,
            ),
            dataAndPrivacyRes = R.string.guide_apps_privacy,
            limitsRes = R.string.guide_apps_limits,
            evaluateBadge = { state ->
                if (!state.elevated && !state.romServiceAvailable) {
                    FeatureLiveBadge.NeedsPowerLevel(R.string.guide_req_elevated)
                } else if (state.devices.none { it.online }) {
                    FeatureLiveBadge.ConnectPcToCheck
                } else {
                    FeatureLiveBadge.Available
                }
            },
            evaluateFixAction = { state ->
                if (!state.elevated && !state.romServiceAvailable) GuideFixAction.ENABLE_ELEVATED else null
            },
        ),

        // 17. Phone as webcam
        FeatureGuideEntry(
            id = "phone_as_pc_webcam",
            area = FeatureArea.MirroringCamera,
            titleRes = R.string.guide_webcam_title,
            summaryRes = R.string.guide_webcam_summary,
            keywords = listOf("phone as webcam", "webcam", "camera", "video", "front", "back", "1080p", "4k"),
            whereTextRes = R.string.guide_webcam_where,
            showMeTarget = GuideHighlightTarget.HOME_PC_CARD,
            needsSummaryRes = R.string.guide_webcam_needs,
            whatYouNeedResIds = listOf(
                R.string.guide_webcam_need_1,
            ),
            stepByStepPhoneResIds = listOf(
                R.string.guide_webcam_phone_1,
            ),
            stepByStepPcResIds = listOf(
                R.string.guide_webcam_pc_1,
            ),
            optionsExplainedResIds = listOf(
                R.string.guide_webcam_opt_1,
            ),
            dataAndPrivacyRes = R.string.guide_webcam_privacy,
            limitsRes = R.string.guide_webcam_limits,
            evaluateBadge = { state ->
                if (state.devices.none { it.online }) {
                    FeatureLiveBadge.ConnectPcToCheck
                } else {
                    FeatureLiveBadge.Available
                }
            },
            evaluateFixAction = { null },
        ),

        // 18. Continuity camera
        FeatureGuideEntry(
            id = "continuity_camera",
            area = FeatureArea.MirroringCamera,
            titleRes = R.string.guide_continuity_title,
            summaryRes = R.string.guide_continuity_summary,
            keywords = listOf("continuity camera", "scan document", "take photo", "camera", "scanner", "crop"),
            whereTextRes = R.string.guide_continuity_where,
            showMeTarget = GuideHighlightTarget.HOME_PC_CARD,
            needsSummaryRes = R.string.guide_continuity_needs,
            whatYouNeedResIds = listOf(
                R.string.guide_continuity_need_1,
            ),
            stepByStepPhoneResIds = listOf(
                R.string.guide_continuity_phone_1,
            ),
            stepByStepPcResIds = listOf(
                R.string.guide_continuity_pc_1,
            ),
            optionsExplainedResIds = listOf(
                R.string.guide_continuity_opt_1,
            ),
            dataAndPrivacyRes = R.string.guide_continuity_privacy,
            limitsRes = R.string.guide_continuity_limits,
            evaluateBadge = { state ->
                if (state.devices.none { it.online }) {
                    FeatureLiveBadge.ConnectPcToCheck
                } else {
                    FeatureLiveBadge.Available
                }
            },
            evaluateFixAction = { null },
        ),

        // 19. Voice recorder
        FeatureGuideEntry(
            id = "voice_recorder",
            area = FeatureArea.MirroringCamera,
            titleRes = R.string.guide_recorder_title,
            summaryRes = R.string.guide_recorder_summary,
            keywords = listOf("voice recorder", "voice", "recorder", "microphone", "audio", "notes", "markers", "m4a"),
            whereTextRes = R.string.guide_recorder_where,
            showMeTarget = GuideHighlightTarget.HOME_PC_CARD,
            needsSummaryRes = R.string.guide_recorder_needs,
            whatYouNeedResIds = listOf(
                R.string.guide_recorder_need_1,
            ),
            stepByStepPhoneResIds = listOf(
                R.string.guide_recorder_phone_1,
                R.string.guide_recorder_phone_2,
            ),
            stepByStepPcResIds = listOf(
                R.string.guide_recorder_pc_1,
            ),
            optionsExplainedResIds = listOf(
                R.string.guide_recorder_opt_1,
            ),
            dataAndPrivacyRes = R.string.guide_recorder_privacy,
            limitsRes = R.string.guide_recorder_limits,
            evaluateBadge = { state ->
                if (state.devices.none { it.online }) {
                    FeatureLiveBadge.ConnectPcToCheck
                } else {
                    FeatureLiveBadge.Available
                }
            },
            evaluateFixAction = { null },
        ),

        // 20. Control PC
        FeatureGuideEntry(
            id = "control_pc_remote",
            area = FeatureArea.InputControl,
            titleRes = R.string.guide_remote_title,
            summaryRes = R.string.guide_remote_summary,
            keywords = listOf("control pc", "touchpad", "trackpad", "mouse", "keyboard", "air mouse", "presentation", "laser", "remote", "dictate"),
            whereTextRes = R.string.guide_remote_where,
            showMeTarget = GuideHighlightTarget.HOME_PC_CARD,
            needsSummaryRes = R.string.guide_remote_needs,
            whatYouNeedResIds = listOf(
                R.string.guide_remote_need_1,
            ),
            stepByStepPhoneResIds = listOf(
                R.string.guide_remote_phone_1,
                R.string.guide_remote_phone_2,
            ),
            stepByStepPcResIds = listOf(
                R.string.guide_remote_pc_1,
            ),
            optionsExplainedResIds = listOf(
                R.string.guide_remote_opt_1,
            ),
            dataAndPrivacyRes = R.string.guide_remote_privacy,
            limitsRes = R.string.guide_remote_limits,
            evaluateBadge = { state ->
                if (state.devices.none { it.online }) {
                    FeatureLiveBadge.ConnectPcToCheck
                } else {
                    FeatureLiveBadge.Available
                }
            },
            evaluateFixAction = { null },
        ),

        // 21. Assist
        FeatureGuideEntry(
            id = "control_phone_from_pc",
            area = FeatureArea.InputControl,
            titleRes = R.string.guide_assist_title,
            summaryRes = R.string.guide_assist_summary,
            keywords = listOf("assist", "accessibility", "input", "type from pc", "mouse", "keyboard", "control from pc"),
            whereTextRes = R.string.guide_assist_where,
            showMeTarget = GuideHighlightTarget.SETTINGS_CONTROL,
            needsSummaryRes = R.string.guide_assist_needs,
            whatYouNeedResIds = listOf(
                R.string.guide_assist_need_1,
            ),
            stepByStepPhoneResIds = listOf(
                R.string.guide_assist_phone_1,
            ),
            stepByStepPcResIds = listOf(
                R.string.guide_assist_pc_1,
            ),
            optionsExplainedResIds = listOf(
                R.string.guide_assist_opt_1,
            ),
            dataAndPrivacyRes = R.string.guide_assist_privacy,
            limitsRes = R.string.guide_assist_limits,
            evaluateBadge = { state ->
                if (!state.inputAccess && !state.elevated) {
                    FeatureLiveBadge.NeedsPowerLevel(R.string.guide_req_assist_or_elevated)
                } else if (state.devices.none { it.online }) {
                    FeatureLiveBadge.ConnectPcToCheck
                } else {
                    FeatureLiveBadge.Available
                }
            },
            evaluateFixAction = { state ->
                if (!state.inputAccess && !state.elevated) GuideFixAction.OPEN_ACCESSIBILITY else null
            },
        ),

        // 22. Elevated
        FeatureGuideEntry(
            id = "elevated_mode",
            area = FeatureArea.InputControl,
            titleRes = R.string.guide_elevated_title,
            summaryRes = R.string.guide_elevated_summary,
            keywords = listOf("elevated", "wireless debugging", "adb", "shizuku", "power level", "background clipboard"),
            whereTextRes = R.string.guide_elevated_where,
            showMeTarget = GuideHighlightTarget.SETTINGS_CONTROL,
            needsSummaryRes = R.string.guide_elevated_needs,
            whatYouNeedResIds = listOf(
                R.string.guide_elevated_need_1,
            ),
            stepByStepPhoneResIds = listOf(
                R.string.guide_elevated_phone_1,
                R.string.guide_elevated_phone_2,
            ),
            stepByStepPcResIds = listOf(
                R.string.guide_elevated_pc_1,
            ),
            optionsExplainedResIds = listOf(
                R.string.guide_elevated_opt_1,
            ),
            dataAndPrivacyRes = R.string.guide_elevated_privacy,
            limitsRes = R.string.guide_elevated_limits,
            evaluateBadge = { state ->
                if (!state.elevated) {
                    FeatureLiveBadge.NeedsPowerLevel(R.string.guide_req_elevated)
                } else {
                    FeatureLiveBadge.Available
                }
            },
            evaluateFixAction = { state ->
                if (!state.elevated) GuideFixAction.ENABLE_ELEVATED else null
            },
        ),

        // 23. Deck
        FeatureGuideEntry(
            id = "deck_control_surface",
            area = FeatureArea.InputControl,
            titleRes = R.string.guide_deck_title,
            summaryRes = R.string.guide_deck_summary,
            keywords = listOf("deck", "stream deck", "macros", "shortcuts", "cpu", "ram", "gauges", "buttons"),
            whereTextRes = R.string.guide_deck_where,
            showMeTarget = GuideHighlightTarget.HOME_PC_CARD,
            needsSummaryRes = R.string.guide_deck_needs,
            whatYouNeedResIds = listOf(
                R.string.guide_deck_need_1,
            ),
            stepByStepPhoneResIds = listOf(
                R.string.guide_deck_phone_1,
            ),
            stepByStepPcResIds = listOf(
                R.string.guide_deck_pc_1,
            ),
            optionsExplainedResIds = listOf(
                R.string.guide_deck_opt_1,
            ),
            dataAndPrivacyRes = R.string.guide_deck_privacy,
            limitsRes = R.string.guide_deck_limits,
            evaluateBadge = { state ->
                if (state.devices.none { it.online }) {
                    FeatureLiveBadge.ConnectPcToCheck
                } else {
                    FeatureLiveBadge.Available
                }
            },
            evaluateFixAction = { null },
        ),

        // 24. Desk Mode
        FeatureGuideEntry(
            id = "desk_mode",
            area = FeatureArea.InputControl,
            titleRes = R.string.guide_desk_title,
            summaryRes = R.string.guide_desk_summary,
            keywords = listOf("desk mode", "desk clock", "standby", "clock", "oled", "nightstand", "always on"),
            whereTextRes = R.string.guide_desk_where,
            showMeTarget = GuideHighlightTarget.HOME_PC_CARD,
            needsSummaryRes = R.string.guide_desk_needs,
            whatYouNeedResIds = listOf(
                R.string.guide_desk_need_1,
            ),
            stepByStepPhoneResIds = listOf(
                R.string.guide_desk_phone_1,
            ),
            stepByStepPcResIds = listOf(
                R.string.guide_desk_pc_1,
            ),
            optionsExplainedResIds = listOf(
                R.string.guide_desk_opt_1,
            ),
            dataAndPrivacyRes = R.string.guide_desk_privacy,
            limitsRes = R.string.guide_desk_limits,
            evaluateBadge = { state ->
                if (state.devices.none { it.online }) {
                    FeatureLiveBadge.ConnectPcToCheck
                } else {
                    FeatureLiveBadge.Available
                }
            },
            evaluateFixAction = { null },
        ),

        // 25. Wake PC
        FeatureGuideEntry(
            id = "ring_lock_sleep_wake",
            area = FeatureArea.PowerNetwork,
            titleRes = R.string.guide_wake_title,
            summaryRes = R.string.guide_wake_summary,
            keywords = listOf("wake pc", "wake", "wol", "wake on lan", "power"),
            whereTextRes = R.string.guide_wake_where,
            showMeTarget = GuideHighlightTarget.HOME_PC_CARD,
            needsSummaryRes = R.string.guide_wake_needs,
            whatYouNeedResIds = listOf(
                R.string.guide_wake_need_1,
                R.string.guide_wake_need_2,
            ),
            stepByStepPhoneResIds = listOf(
                R.string.guide_wake_phone_1,
            ),
            stepByStepPcResIds = listOf(
                R.string.guide_wake_pc_1,
            ),
            optionsExplainedResIds = listOf(
                R.string.guide_wake_opt_1,
            ),
            dataAndPrivacyRes = R.string.guide_wake_privacy,
            limitsRes = R.string.guide_wake_limits,
            evaluateBadge = { state ->
                if (state.devices.none { it.online }) {
                    FeatureLiveBadge.ConnectPcToCheck
                } else {
                    FeatureLiveBadge.Available
                }
            },
            evaluateFixAction = { null },
        ),

        // 26. Quick actions
        FeatureGuideEntry(
            id = "phone_quick_toggles",
            area = FeatureArea.PowerNetwork,
            titleRes = R.string.guide_quick_title,
            summaryRes = R.string.guide_quick_summary,
            keywords = listOf("quick actions", "toggles", "ring", "lock", "sleep", "dnd", "do not disturb", "flashlight", "brightness", "widget"),
            whereTextRes = R.string.guide_quick_where,
            showMeTarget = GuideHighlightTarget.SETTINGS_CONTROL,
            needsSummaryRes = R.string.guide_quick_needs,
            whatYouNeedResIds = listOf(
                R.string.guide_quick_need_1,
                R.string.guide_quick_need_2,
                R.string.guide_quick_need_3,
            ),
            stepByStepPhoneResIds = listOf(
                R.string.guide_quick_phone_1,
            ),
            stepByStepPcResIds = listOf(
                R.string.guide_quick_pc_1,
            ),
            optionsExplainedResIds = listOf(
                R.string.guide_quick_opt_1,
            ),
            dataAndPrivacyRes = R.string.guide_quick_privacy,
            limitsRes = R.string.guide_quick_limits,
            evaluateBadge = { state ->
                if (!state.dndAccess) {
                    FeatureLiveBadge.NeedsPermission(R.string.guide_req_dnd)
                } else if (!state.writeSettingsAccess && !state.elevated) {
                    FeatureLiveBadge.NeedsPermission(R.string.guide_req_write_settings)
                } else if (state.devices.none { it.online }) {
                    FeatureLiveBadge.ConnectPcToCheck
                } else {
                    FeatureLiveBadge.Available
                }
            },
            evaluateFixAction = { state ->
                when {
                    !state.dndAccess -> GuideFixAction.ENABLE_DND
                    !state.writeSettingsAccess && !state.elevated -> GuideFixAction.ENABLE_WRITE_SETTINGS
                    else -> null
                }
            },
        ),

        // 27. Connect over mobile data
        FeatureGuideEntry(
            id = "away_mode_and_usb",
            area = FeatureArea.PowerNetwork,
            titleRes = R.string.guide_away_title,
            summaryRes = R.string.guide_away_summary,
            keywords = listOf("connect over mobile data", "mobile data", "lte", "5g", "relay", "usb", "tethering", "adb", "network"),
            whereTextRes = R.string.guide_away_where,
            showMeTarget = GuideHighlightTarget.SETTINGS_DEVICES,
            needsSummaryRes = R.string.guide_away_needs,
            whatYouNeedResIds = listOf(
                R.string.guide_away_need_1,
                R.string.guide_away_need_2,
            ),
            stepByStepPhoneResIds = listOf(
                R.string.guide_away_phone_1,
            ),
            stepByStepPcResIds = listOf(
                R.string.guide_away_pc_1,
            ),
            optionsExplainedResIds = listOf(
                R.string.guide_away_opt_1,
            ),
            dataAndPrivacyRes = R.string.guide_away_privacy,
            limitsRes = R.string.guide_away_limits,
            evaluateBadge = { state ->
                if (!state.backgroundUnrestricted) {
                    FeatureLiveBadge.NeedsPermission(R.string.guide_req_unrestricted_battery)
                } else if (state.devices.none { it.online }) {
                    FeatureLiveBadge.ConnectPcToCheck
                } else {
                    FeatureLiveBadge.Available
                }
            },
            evaluateFixAction = { state ->
                if (!state.backgroundUnrestricted) GuideFixAction.GRANT_BATTERY else null
            },
        ),

        // 28. Connection Doctor
        FeatureGuideEntry(
            id = "connection_doctor",
            area = FeatureArea.PowerNetwork,
            titleRes = R.string.guide_doctor_title,
            summaryRes = R.string.guide_doctor_summary,
            keywords = listOf("connection doctor", "battery", "background", "companion", "ble", "diagnostics", "troubleshooting"),
            whereTextRes = R.string.guide_doctor_where,
            showMeTarget = GuideHighlightTarget.SETTINGS_BACKGROUND,
            needsSummaryRes = R.string.guide_doctor_needs,
            whatYouNeedResIds = listOf(
                R.string.guide_doctor_need_1,
                R.string.guide_doctor_need_2,
            ),
            stepByStepPhoneResIds = listOf(
                R.string.guide_doctor_phone_1,
                R.string.guide_doctor_phone_2,
            ),
            stepByStepPcResIds = listOf(
                R.string.guide_doctor_pc_1,
            ),
            optionsExplainedResIds = listOf(
                R.string.guide_doctor_opt_1,
            ),
            dataAndPrivacyRes = R.string.guide_doctor_privacy,
            limitsRes = R.string.guide_doctor_limits,
            evaluateBadge = { state ->
                if (!state.backgroundUnrestricted) {
                    FeatureLiveBadge.NeedsPermission(R.string.guide_req_unrestricted_battery)
                } else if (state.devices.none { it.online }) {
                    FeatureLiveBadge.ConnectPcToCheck
                } else {
                    FeatureLiveBadge.Available
                }
            },
            evaluateFixAction = { state ->
                if (!state.backgroundUnrestricted) GuideFixAction.GRANT_BATTERY else null
            },
        ),
    )

    fun filter(
        context: Context,
        area: FeatureArea,
        searchQuery: String,
        includeElevated: Boolean = true,
    ): List<FeatureGuideEntry> {
        val q = searchQuery.trim().lowercase()
        return entries.filter { entry ->
            val allowed = includeElevated || entry.id != "elevated_mode"
            val matchesArea = area == FeatureArea.All || entry.area == area
            val matchesQuery = q.isEmpty() ||
                context.getString(entry.titleRes).lowercase().contains(q) ||
                context.getString(entry.summaryRes).lowercase().contains(q) ||
                context.getString(entry.whereTextRes).lowercase().contains(q) ||
                context.getString(entry.needsSummaryRes).lowercase().contains(q) ||
                entry.keywords.any { it.lowercase().contains(q) }
            allowed && matchesArea && matchesQuery
        }
    }

    fun filter(
        area: FeatureArea,
        searchQuery: String,
        includeElevated: Boolean = true,
    ): List<FeatureGuideEntry> {
        val q = searchQuery.trim().lowercase()
        return entries.filter { entry ->
            val allowed = includeElevated || entry.id != "elevated_mode"
            val matchesArea = area == FeatureArea.All || entry.area == area
            val matchesQuery = q.isEmpty() ||
                entry.id.lowercase().contains(q) ||
                entry.keywords.any { it.lowercase().contains(q) }
            allowed && matchesArea && matchesQuery
        }
    }
}
