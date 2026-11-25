// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.storage

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.result.contract.ActivityResultContracts
import app.nectarlink.android.NectarlinkApplication

/**
 * Opens Android's folder picker and shares the folder picked with paired PCs,
 * then closes. No screen of its own.
 *
 * Where the GitHub build asks for All files access, the Google Play build
 * opens this instead: Play allows All files access to file managers only, and
 * picked folders already cover phone storage on the PC
 * ([PhoneStorage.allFilesAccessIntent]).
 */
class PickFolderActivity : ComponentActivity() {
    private val pick = registerForActivityResult(ActivityResultContracts.OpenDocumentTree()) { tree ->
        if (tree != null) (application as NectarlinkApplication).core.addSafFolder(tree)
        finish()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Recreated (rotated) while the picker is open: its answer still comes here.
        if (savedInstanceState == null) {
            runCatching { pick.launch(null) }.onFailure { finish() }
        }
    }
}
