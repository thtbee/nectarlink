// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.net.Uri
import android.os.ParcelFileDescriptor
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import app.nectarlink.android.R
import app.nectarlink.android.core.Core
import app.nectarlink.android.files.OutgoingFiles
import app.nectarlink.android.files.ReceivedFiles
import app.nectarlink.android.ui.pairing.QrCode
import app.nectarlink.core.AnySendEvent
import app.nectarlink.core.AnySendRoute
import app.nectarlink.core.AnySendSession
import app.nectarlink.core.FileToSend
import java.io.File
import java.net.Inet4Address
import java.nio.file.Files
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext

private data class AnySendRequest(val file: Uri?, val site: String, val limit: Int)

@Composable
internal fun AnySendDialog(core: Core, onDismiss: () -> Unit) {
    val context = LocalContext.current
    val prefs = remember(context) { context.getSharedPreferences("anysend", Context.MODE_PRIVATE) }
    var site by remember { mutableStateOf(prefs.getString("site", "").orEmpty()) }
    var limit by remember { mutableStateOf("1") }
    var request by remember { mutableStateOf<AnySendRequest?>(null) }
    var active by remember { mutableStateOf(false) }
    var finished by remember { mutableStateOf(false) }
    val tickets = remember { mutableStateMapOf<AnySendRoute, String>() }
    val unavailable = remember { mutableStateMapOf<AnySendRoute, String>() }
    var activeRoute by remember { mutableStateOf<AnySendRoute?>(null) }
    var progress by remember { mutableStateOf<Float?>(null) }
    var connected by remember { mutableStateOf(0) }
    var completed by remember { mutableStateOf(0) }
    var failed by remember { mutableStateOf(0) }
    var saved by remember { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var saveFailed by remember { mutableStateOf(false) }

    val browsers = limit.toIntOrNull()?.takeIf { it in 1..100 }
    fun start(file: Uri?) {
        val count = browsers ?: return
        prefs.edit().putString("site", site.trim()).apply()
        request = AnySendRequest(file, site.trim(), count)
    }
    val pick = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        if (uri != null) start(uri)
    }

    KeepScreenOn(active)
    LaunchedEffect(request) {
        val transfer = request ?: return@LaunchedEffect
        active = true
        var directory: File? = null
        var session: AnySendSession? = null
        val failedMessage = context.getString(R.string.transfer_failed)
        suspend fun publish(path: String): String? = withContext(NonCancellable + Dispatchers.IO) {
            val file = File(path)
            if (ReceivedFiles.publish(context, file).isNotEmpty()) file.name else null
        }
        try {
            withContext(Dispatchers.IO) {
                val folder = Files.createTempDirectory(context.cacheDir.toPath(), "anysend-").toFile()
                directory = folder
                val lanIp = anySendLanIp(context)
                val file = transfer.file?.let { openAnySendSource(context, it, File(folder, "outgoing")) }
                session = core.startAnySend(
                    directory = folder.absolutePath,
                    file = file,
                    site = transfer.site,
                    lanIp = lanIp,
                    limit = transfer.limit,
                )
            }
            val current = checkNotNull(session)
            while (true) {
                when (val event = current.nextEvent() ?: break) {
                    is AnySendEvent.Ticket -> {
                        unavailable.remove(event.route)
                        tickets[event.route] = event.url
                    }
                    is AnySendEvent.Unavailable -> {
                        tickets.remove(event.route)
                        unavailable[event.route] = event.reason
                        if (!finished && transfer.limit == 1 && event.route == activeRoute) {
                            error = event.reason
                            break
                        }
                    }
                    is AnySendEvent.Started -> {
                        activeRoute = event.route
                        progress = 0f
                    }
                    is AnySendEvent.Guests -> {
                        connected = event.connected.toInt()
                        completed = event.completed.toInt()
                        failed = event.failed.toInt()
                    }
                    is AnySendEvent.Progress -> {
                        progress = if (event.total == 0uL) 1f else (event.bytes.toDouble() / event.total.toDouble()).toFloat()
                    }
                    is AnySendEvent.Received -> {
                        val name = publish(event.path)
                        if (name == null) saveFailed = true else saved = name
                    }
                    AnySendEvent.Finished -> {
                        finished = true
                        active = false
                        tickets.clear()
                    }
                    is AnySendEvent.Failed -> {
                        error = event.reason
                        break
                    }
                    AnySendEvent.Expired -> {
                        if (!finished) error = context.getString(R.string.anysend_expired)
                        break
                    }
                }
            }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            error = e.localizedMessage ?: failedMessage
        } finally {
            withContext(NonCancellable + Dispatchers.IO) {
                session?.let { current ->
                    current.stop()
                    // Publish transfers that finished as the dialog was closing.
                    while (true) {
                        val event = current.nextEvent() ?: break
                        if (event is AnySendEvent.Received) publish(event.path)
                    }
                    current.close()
                }
                if (transfer.file != null) directory?.deleteRecursively() else directory?.delete()
            }
            active = false
            tickets.clear()
        }
    }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("AnySend") },
        text = {
            Column(
                modifier = Modifier.fillMaxWidth().verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                val transfer = request
                if (transfer == null) {
                    OutlinedTextField(
                        value = site,
                        onValueChange = { site = it },
                        label = { Text(stringResource(R.string.anysend_site)) },
                        placeholder = { Text("anysend-nectarlink.netlify.app") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                    )
                    OutlinedTextField(
                        value = limit,
                        onValueChange = { limit = it.filter(Char::isDigit).take(3) },
                        label = { Text(stringResource(R.string.anysend_browsers)) },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                        isError = browsers == null,
                    )
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Button(onClick = { pick.launch(arrayOf("*/*")) }, enabled = browsers != null) {
                            Text(stringResource(R.string.action_send_files))
                        }
                        OutlinedButton(onClick = { start(null) }, enabled = browsers != null) {
                            Text(stringResource(R.string.anysend_receive))
                        }
                    }
                } else {
                    if (connected < transfer.limit && error == null) {
                        AnySendCode(stringResource(R.string.anysend_lan), tickets[AnySendRoute.LAN], unavailable[AnySendRoute.LAN])
                        AnySendCode(stringResource(R.string.anysend_global), tickets[AnySendRoute.RELAY], unavailable[AnySendRoute.RELAY])
                    }
                    if (transfer.limit > 1 || finished) {
                        Text(stringResource(R.string.anysend_completed, completed, transfer.limit))
                    }
                    if (active && transfer.limit == 1) {
                        progress?.let { fraction ->
                            LinearProgressIndicator(progress = { fraction }, modifier = Modifier.fillMaxWidth())
                        }
                    }
                    saved?.let { Text(stringResource(R.string.pc_files_saved, it)) }
                    error?.let { Text(it) }
                    if (saveFailed || failed > 0 && error == null) Text(stringResource(R.string.transfer_failed))
                }
            }
        },
        confirmButton = {
            TextButton(onClick = onDismiss) {
                Text(stringResource(if (active) R.string.action_stop else R.string.action_done))
            }
        },
    )
}

@Composable
private fun AnySendCode(label: String, ticket: String?, unavailable: String?) {
    Text(label)
    when {
        ticket != null -> Box(
            modifier = Modifier.fillMaxWidth(),
            contentAlignment = Alignment.Center,
        ) {
            Box(Modifier.size(208.dp).background(Color.White).padding(12.dp).semantics { contentDescription = label }) {
                QrCode(ticket)
            }
        }
        unavailable != null -> Text(unavailable)
        else -> CircularProgressIndicator(Modifier.size(24.dp))
    }
}

private fun anySendLanIp(context: Context): String? {
    val manager = context.getSystemService(ConnectivityManager::class.java) ?: return null
    val networks = listOfNotNull(manager.activeNetwork) + manager.allNetworks.toList()
    for (network in networks.distinct()) {
        val caps = manager.getNetworkCapabilities(network) ?: continue
        if (!caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) &&
            !caps.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)
        ) continue
        val address = manager.getLinkProperties(network)?.linkAddresses?.firstOrNull {
            val ip = it.address
            ip is Inet4Address && !ip.isLoopbackAddress && !ip.isLinkLocalAddress
        }
        if (address != null) return address.address.hostAddress
    }
    return null
}

private suspend fun openAnySendSource(context: Context, uri: Uri, target: File): FileToSend {
    check(OutgoingFiles.isAllowedShareUri(uri.scheme, uri.authority, context.packageName))
    val name = OutgoingFiles.displayName(context, uri)
    val fd = checkNotNull(context.contentResolver.openFileDescriptor(uri, "r"))
    if (!OutgoingFiles.isSafeOpenedFd(context, fd)) {
        fd.close()
        error("The selected file is unavailable.")
    }
    if (fd.statSize >= 0) {
        return FileToSend.Fd(name = name, fd = fd.detachFd(), folder = null)
    }
    // Providers that offer only a stream need a seekable copy for chunk retries.
    val coroutine = currentCoroutineContext()
    ParcelFileDescriptor.AutoCloseInputStream(fd).use { input ->
        target.outputStream().use { output ->
            val buffer = ByteArray(256 * 1024)
            while (true) {
                coroutine.ensureActive()
                val count = input.read(buffer)
                if (count < 0) break
                output.write(buffer, 0, count)
            }
        }
    }
    return FileToSend.Path(name = name, path = target.absolutePath, folder = null)
}
