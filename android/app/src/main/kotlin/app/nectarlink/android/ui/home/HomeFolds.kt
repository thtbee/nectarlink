// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.home

import android.content.Context
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.collapse
import androidx.compose.ui.semantics.expand
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * The sections of a PC's card on Home the user folded (#659), the same on
 * every PC's card and kept on this phone.
 */
internal object HomeFolds {
    private const val PREFS = "home"
    private const val KEY = "folded_sections"

    const val SCREEN = "screen"
    const val TRANSFER = "transfer"
    const val SYSTEM = "system"

    @Volatile
    private var folded: MutableStateFlow<Set<String>>? = null

    fun state(context: Context): StateFlow<Set<String>> = flow(context).asStateFlow()

    fun toggle(context: Context, section: String) {
        val flow = flow(context)
        val next = if (section in flow.value) flow.value - section else flow.value + section
        flow.value = next
        context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .edit().putStringSet(KEY, next).apply()
    }

    private fun flow(context: Context): MutableStateFlow<Set<String>> =
        folded ?: synchronized(this) {
            folded ?: MutableStateFlow(
                context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
                    .getStringSet(KEY, emptySet()).orEmpty().toSet(),
            ).also { folded = it }
        }
}

/**
 * A section's title that folds or unfolds it on a tap. TalkBack reads it as
 * a heading and says whether it's expanded or collapsed.
 */
@Composable
internal fun FoldableSectionTitle(
    title: String,
    folded: Boolean,
    color: Color,
    chevron: ImageVector,
    onToggle: () -> Unit,
    top: Dp = 0.dp,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(top = top)
            .clip(RoundedCornerShape(8.dp))
            .clickable(role = Role.Button, onClick = onToggle)
            .semantics(mergeDescendants = true) {
                heading()
                if (folded) {
                    expand { onToggle(); true }
                } else {
                    collapse { onToggle(); true }
                }
            }
            .padding(start = 2.dp, top = 4.dp, bottom = 4.dp, end = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = title,
            style = MaterialTheme.typography.labelSmall,
            color = color,
            modifier = Modifier.weight(1f),
        )
        Icon(
            imageVector = chevron,
            contentDescription = null,
            tint = color,
            modifier = Modifier.size(16.dp).rotate(if (folded) -90f else 0f),
        )
    }
}
