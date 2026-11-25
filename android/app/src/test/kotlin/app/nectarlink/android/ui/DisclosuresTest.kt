// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui

import android.Manifest
import app.nectarlink.android.ui.Disclosures.Kind
import org.junit.Assert.assertEquals
import org.junit.Test

class DisclosuresTest {
    @Test
    fun each_permission_has_the_disclosure_for_its_data() {
        assertEquals(
            listOf(Kind.Messages, Kind.Contacts),
            Disclosures.kindsOf(arrayOf(Manifest.permission.READ_SMS, Manifest.permission.SEND_SMS, Manifest.permission.READ_CONTACTS)),
        )
        assertEquals(
            listOf(Kind.Calls),
            Disclosures.kindsOf(arrayOf(Manifest.permission.READ_PHONE_STATE, Manifest.permission.READ_CALL_LOG, Manifest.permission.ANSWER_PHONE_CALLS)),
        )
        assertEquals(
            listOf(Kind.Photos),
            Disclosures.kindsOf(arrayOf(Manifest.permission.READ_MEDIA_IMAGES, Manifest.permission.READ_MEDIA_VIDEO)),
        )
    }

    @Test
    fun permissions_that_share_nothing_need_none() {
        assertEquals(emptyList<Kind>(), Disclosures.kindsOf(arrayOf(Manifest.permission.CAMERA, Manifest.permission.RECORD_AUDIO)))
    }
}
