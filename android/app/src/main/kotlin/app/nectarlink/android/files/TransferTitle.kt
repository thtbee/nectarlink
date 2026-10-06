// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.files

import android.content.res.Resources
import app.nectarlink.android.R
import app.nectarlink.core.Transfer

/** What a transfer is called: its file or folder's name, or how many. */
internal fun transferTitle(resources: Resources, transfer: Transfer): String {
    val count = transfer.names.size
    return when {
        count == 1 -> transfer.names[0]
        transfer.files.toInt() == count -> resources.getQuantityString(R.plurals.transfer_files, count, count)
        else -> resources.getQuantityString(R.plurals.transfer_items, count, count)
    }
}
