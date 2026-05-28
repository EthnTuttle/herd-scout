package com.herdscout.admin

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Bundle
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.viewModels
import androidx.core.content.ContextCompat
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.AssistChip
import androidx.compose.material3.AssistChipDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Tab
import androidx.compose.material3.TabRow
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.paging.compose.collectAsLazyPagingItems
import androidx.paging.compose.itemKey
import com.herdscout.shared.NodeIdFormat
import com.herdscout.shared.QrScanActivity

class AdminActivity : ComponentActivity() {

    private val vm: AdminViewModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            AdminTheme {
                Surface(modifier = Modifier.fillMaxSize()) {
                    AdminScreen(vm)
                }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun AdminScreen(vm: AdminViewModel) {
    val daemons by vm.daemons.collectAsStateWithLifecycle()
    val activeId by vm.activeDaemonId.collectAsStateWithLifecycle()
    val connection by vm.connection.collectAsStateWithLifecycle()
    val toast by vm.toast.collectAsStateWithLifecycle()
    val ctx = LocalContext.current

    LaunchedEffect(toast) {
        toast?.let {
            Toast.makeText(ctx, it, Toast.LENGTH_LONG).show()
            vm.consumeToast()
        }
    }

    var tabIndex by remember { mutableIntStateOf(0) }
    var switcherOpen by remember { mutableStateOf(false) }
    var addDaemonOpen by remember { mutableStateOf(false) }

    val activeEntry = daemons.firstOrNull { it.nodeId == activeId }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.admin_app_name)) },
                actions = {
                    DaemonChip(
                        active = activeEntry,
                        connection = connection,
                        onClick = { switcherOpen = true },
                    )
                    Spacer(Modifier.width(4.dp))
                    IconButton(onClick = { vm.refreshAll() }) {
                        Icon(Icons.Filled.Refresh, contentDescription = "Refresh")
                    }
                },
                colors = TopAppBarDefaults.topAppBarColors(
                    containerColor = MaterialTheme.colorScheme.primary,
                    titleContentColor = MaterialTheme.colorScheme.onPrimary,
                    actionIconContentColor = MaterialTheme.colorScheme.onPrimary,
                ),
            )
        },
    ) { padding ->
        Column(modifier = Modifier.padding(padding).fillMaxSize()) {
            TabRow(selectedTabIndex = tabIndex) {
                listOf(
                    R.string.tab_allowlist,
                    R.string.tab_history,
                    R.string.tab_identity,
                ).forEachIndexed { i, label ->
                    Tab(
                        selected = tabIndex == i,
                        onClick = { tabIndex = i },
                        text = { Text(stringResource(label)) },
                    )
                }
            }
            when (tabIndex) {
                0 -> AllowlistTab(vm)
                1 -> HistoryTab(vm)
                else -> IdentityTab(vm)
            }
        }
    }

    if (switcherOpen) {
        DaemonSwitcherSheet(
            daemons = daemons,
            activeId = activeId,
            onSelect = {
                vm.selectDaemon(it.nodeId)
                switcherOpen = false
            },
            onForget = { vm.forgetDaemon(it.nodeId) },
            onAdd = {
                switcherOpen = false
                addDaemonOpen = true
            },
            onDismiss = { switcherOpen = false },
        )
    }

    if (addDaemonOpen) {
        AddDaemonDialog(
            onDismiss = { addDaemonOpen = false },
            onSave = { label, nodeId ->
                vm.addDaemon(label, nodeId)
                vm.selectDaemon(nodeId)
                addDaemonOpen = false
            },
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun DaemonChip(
    active: DaemonEntry?,
    connection: AdminViewModel.ConnectionState,
    onClick: () -> Unit,
) {
    val (label, indicator) = when {
        active == null -> stringResource(R.string.no_daemon_selected) to "·"
        connection == AdminViewModel.ConnectionState.Connecting -> active.label to "↻"
        connection == AdminViewModel.ConnectionState.Connected -> active.label to "●"
        else -> active.label to "○"
    }
    AssistChip(
        onClick = onClick,
        label = { Text("$indicator $label", maxLines = 1, overflow = TextOverflow.Ellipsis) },
        trailingIcon = {
            Icon(Icons.Filled.ArrowDropDown, contentDescription = null)
        },
        colors = AssistChipDefaults.assistChipColors(
            containerColor = MaterialTheme.colorScheme.onPrimary.copy(alpha = 0.15f),
            labelColor = MaterialTheme.colorScheme.onPrimary,
            trailingIconContentColor = MaterialTheme.colorScheme.onPrimary,
        ),
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun DaemonSwitcherSheet(
    daemons: List<DaemonEntry>,
    activeId: String?,
    onSelect: (DaemonEntry) -> Unit,
    onForget: (DaemonEntry) -> Unit,
    onAdd: () -> Unit,
    onDismiss: () -> Unit,
) {
    val sheetState = rememberModalBottomSheetState()
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = sheetState) {
        Column(modifier = Modifier.padding(16.dp)) {
            Text("Daemons", style = MaterialTheme.typography.titleMedium)
            Spacer(Modifier.height(8.dp))
            daemons.forEach { d ->
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .clickable { onSelect(d) }
                        .padding(vertical = 12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        if (d.nodeId == activeId) "●" else "○",
                        modifier = Modifier.width(24.dp),
                    )
                    Column(modifier = Modifier.weight(1f)) {
                        Text(d.label, style = MaterialTheme.typography.bodyLarge)
                        Text(
                            NodeIdFormat.short(d.nodeId),
                            style = MaterialTheme.typography.bodySmall,
                            fontFamily = FontFamily.Monospace,
                        )
                    }
                    IconButton(onClick = { onForget(d) }) {
                        Icon(Icons.Filled.Delete, contentDescription = "Forget")
                    }
                }
                HorizontalDivider()
            }
            Spacer(Modifier.height(8.dp))
            TextButton(onClick = onAdd) {
                Icon(Icons.Filled.Add, contentDescription = null)
                Spacer(Modifier.width(8.dp))
                Text(stringResource(R.string.add_daemon))
            }
        }
    }
}

/**
 * Returns a launch lambda that opens [QrScanActivity], requesting the
 * runtime CAMERA permission first if it hasn't been granted. Without this
 * gate the camera-bind inside the scanner throws SecurityException and
 * crashes the activity.
 */
@Composable
private fun rememberQrScanWithPermission(onResult: (String) -> Unit): () -> Unit {
    val ctx = LocalContext.current
    val scanLauncher = rememberLauncherForActivityResult(QrScanActivity.Companion.Contract()) {
        if (!it.isNullOrBlank()) onResult(it.trim())
    }
    val permissionLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestPermission()
    ) { granted ->
        if (granted) {
            scanLauncher.launch(Unit)
        } else {
            Toast.makeText(ctx, ctx.getString(R.string.camera_permission_required), Toast.LENGTH_LONG).show()
        }
    }
    return {
        if (ContextCompat.checkSelfPermission(ctx, Manifest.permission.CAMERA)
            == PackageManager.PERMISSION_GRANTED
        ) {
            scanLauncher.launch(Unit)
        } else {
            permissionLauncher.launch(Manifest.permission.CAMERA)
        }
    }
}

@Composable
private fun AddDaemonDialog(onDismiss: () -> Unit, onSave: (String, String) -> Unit) {
    var label by remember { mutableStateOf("") }
    var nodeId by remember { mutableStateOf("") }
    val launchScan = rememberQrScanWithPermission { nodeId = it }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.add_daemon_title)) },
        text = {
            Column {
                OutlinedTextField(
                    value = label,
                    onValueChange = { label = it },
                    label = { Text(stringResource(R.string.add_daemon_label_hint)) },
                    singleLine = true,
                )
                Spacer(Modifier.height(8.dp))
                OutlinedTextField(
                    value = nodeId,
                    onValueChange = { nodeId = it },
                    label = { Text(stringResource(R.string.add_daemon_node_id_hint)) },
                    singleLine = false,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Ascii),
                )
                Spacer(Modifier.height(8.dp))
                TextButton(onClick = { launchScan() }) {
                    Text(stringResource(R.string.action_scan_qr))
                }
            }
        },
        confirmButton = {
            TextButton(
                onClick = { onSave(label, nodeId) },
                enabled = label.isNotBlank() && nodeId.isNotBlank(),
            ) { Text(stringResource(R.string.action_save)) }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text(stringResource(R.string.action_cancel)) }
        },
    )
}

@Composable
private fun AllowlistTab(vm: AdminViewModel) {
    val allowed by vm.allowed.collectAsStateWithLifecycle()
    val status by vm.status.collectAsStateWithLifecycle()
    var addOpen by remember { mutableStateOf(false) }
    var removeTarget by remember { mutableStateOf<AllowedEntry?>(null) }

    Box(modifier = Modifier.fillMaxSize()) {
        Column(modifier = Modifier.fillMaxSize()) {
            StatusCard(status = status)
            HorizontalDivider()
            if (allowed.isEmpty()) {
                Box(modifier = Modifier.fillMaxSize().padding(32.dp), contentAlignment = Alignment.Center) {
                    Text(
                        "No devices allowed yet. Tap + to add the first one.",
                        style = MaterialTheme.typography.bodyMedium,
                    )
                }
            } else {
                LazyColumn(modifier = Modifier.fillMaxSize()) {
                    items(allowed, key = { it.nodeId }) { entry ->
                        AllowedRow(entry = entry, onLongPress = { removeTarget = entry })
                        HorizontalDivider()
                    }
                }
            }
        }
        FloatingActionButton(
            onClick = { addOpen = true },
            modifier = Modifier
                .align(Alignment.BottomEnd)
                .padding(16.dp),
        ) {
            Icon(Icons.Filled.Add, contentDescription = "Add")
        }
    }

    if (addOpen) {
        AddAllowedDialog(
            onDismiss = { addOpen = false },
            onSave = { node, label ->
                vm.addAllowed(node, label)
                addOpen = false
            },
        )
    }

    removeTarget?.let { target ->
        AlertDialog(
            onDismissRequest = { removeTarget = null },
            title = { Text(stringResource(R.string.confirm_remove_title)) },
            text = {
                Column {
                    Text(stringResource(R.string.confirm_remove_message))
                    Spacer(Modifier.height(8.dp))
                    Text(target.label, style = MaterialTheme.typography.titleSmall)
                    Text(
                        NodeIdFormat.short(target.nodeId),
                        style = MaterialTheme.typography.bodySmall,
                        fontFamily = FontFamily.Monospace,
                    )
                }
            },
            confirmButton = {
                TextButton(onClick = {
                    vm.removeAllowed(target.nodeId)
                    removeTarget = null
                }) { Text(stringResource(R.string.action_remove)) }
            },
            dismissButton = {
                TextButton(onClick = { removeTarget = null }) {
                    Text(stringResource(R.string.action_cancel))
                }
            },
        )
    }
}

@Composable
private fun StatusCard(status: StatusReply?) {
    Card(
        modifier = Modifier.fillMaxWidth().padding(16.dp),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.primaryContainer),
    ) {
        Column(modifier = Modifier.padding(16.dp)) {
            if (status == null) {
                Text(stringResource(R.string.status_unknown))
                return@Column
            }
            Text(
                NodeIdFormat.short(status.ownNodeId),
                style = MaterialTheme.typography.titleMedium,
                fontFamily = FontFamily.Monospace,
            )
            Spacer(Modifier.height(4.dp))
            Text(
                "v${status.daemonVersion} · ${status.adminsCount} admins · " +
                    "${status.allowedCount} allowed · ${status.activeSshSessions} ssh",
                style = MaterialTheme.typography.bodySmall,
            )
            Spacer(Modifier.height(2.dp))
            Text(
                "last reload (${status.lastReloadSource}): ${NodeIdFormat.relative(status.lastReloadUnixMs)}",
                style = MaterialTheme.typography.bodySmall,
            )
        }
    }
}

@Composable
private fun AllowedRow(entry: AllowedEntry, onLongPress: () -> Unit) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onLongPress)
            .padding(16.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text(
                entry.label.ifBlank { "(unlabeled)" },
                style = MaterialTheme.typography.bodyLarge,
                fontWeight = FontWeight.Medium,
            )
            Text(
                NodeIdFormat.short(entry.nodeId),
                style = MaterialTheme.typography.bodySmall,
                fontFamily = FontFamily.Monospace,
            )
        }
        IconButton(onClick = onLongPress) {
            Icon(Icons.Filled.Delete, contentDescription = "Remove")
        }
    }
}

@Composable
private fun AddAllowedDialog(
    onDismiss: () -> Unit,
    onSave: (nodeId: String, label: String) -> Unit,
) {
    var nodeId by remember { mutableStateOf("") }
    var label by remember { mutableStateOf("") }
    val launchScan = rememberQrScanWithPermission { nodeId = it }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.add_allowed_title)) },
        text = {
            Column {
                OutlinedTextField(
                    value = label,
                    onValueChange = { label = it },
                    label = { Text(stringResource(R.string.add_allowed_label_hint)) },
                    singleLine = true,
                )
                Spacer(Modifier.height(8.dp))
                OutlinedTextField(
                    value = nodeId,
                    onValueChange = { nodeId = it },
                    label = { Text(stringResource(R.string.add_allowed_node_id_hint)) },
                )
                Spacer(Modifier.height(8.dp))
                TextButton(onClick = { launchScan() }) {
                    Text(stringResource(R.string.action_scan_qr))
                }
            }
        },
        confirmButton = {
            TextButton(
                onClick = { onSave(nodeId, label) },
                enabled = nodeId.isNotBlank() && label.isNotBlank(),
            ) { Text(stringResource(R.string.action_save)) }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text(stringResource(R.string.action_cancel)) }
        },
    )
}

@Composable
private fun HistoryTab(vm: AdminViewModel) {
    var subTab by remember { mutableIntStateOf(0) }
    Column(modifier = Modifier.fillMaxSize()) {
        TabRow(selectedTabIndex = subTab) {
            Tab(
                selected = subTab == 0,
                onClick = { subTab = 0 },
                text = { Text(stringResource(R.string.history_subtab_local)) },
            )
            Tab(
                selected = subTab == 1,
                onClick = { subTab = 1 },
                text = { Text(stringResource(R.string.history_subtab_daemon)) },
            )
        }
        if (subTab == 0) LocalHistoryList(vm) else DaemonHistoryList(vm)
    }
}

@Composable
private fun LocalHistoryList(vm: AdminViewModel) {
    val items = vm.localHistory.collectAsLazyPagingItems()
    LazyColumn(modifier = Modifier.fillMaxSize()) {
        items(
            count = items.itemCount,
            key = items.itemKey { it.id },
        ) { idx ->
            val ev = items[idx] ?: return@items
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 8.dp),
            ) {
                Text(
                    NodeIdFormat.relative(ev.tsMs),
                    style = MaterialTheme.typography.labelSmall,
                    modifier = Modifier.width(80.dp),
                )
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        text = ev.kind + (ev.op?.let { " · $it" } ?: ""),
                        style = MaterialTheme.typography.bodyMedium,
                    )
                    val target = ev.targetLabel?.takeIf { it.isNotBlank() }
                        ?: ev.targetNodeId?.let { NodeIdFormat.short(it) }
                    if (target != null) {
                        Text(target, style = MaterialTheme.typography.bodySmall)
                    }
                    if (!ev.errorCode.isNullOrBlank()) {
                        Text(
                            "${ev.errorCode}: ${ev.errorMessage.orEmpty()}",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.error,
                        )
                    }
                }
            }
            HorizontalDivider()
        }
    }
}

@Composable
private fun DaemonHistoryList(vm: AdminViewModel) {
    val records by vm.daemonHistory.collectAsStateWithLifecycle()
    if (records.isEmpty()) {
        Box(modifier = Modifier.fillMaxSize().padding(32.dp), contentAlignment = Alignment.Center) {
            Text("No records yet — tap refresh once connected.")
        }
        return
    }
    LazyColumn(modifier = Modifier.fillMaxSize()) {
        items(records, key = { "${it.tsMs}-${it.kind}-${it.actorNodeId.orEmpty()}" }) { rec ->
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 8.dp),
            ) {
                Text(
                    NodeIdFormat.relative(rec.tsMs),
                    style = MaterialTheme.typography.labelSmall,
                    modifier = Modifier.width(80.dp),
                )
                Column(modifier = Modifier.weight(1f)) {
                    Text(rec.kind, style = MaterialTheme.typography.bodyMedium)
                    rec.actorNodeId?.let {
                        Text(
                            "by ${NodeIdFormat.short(it)}",
                            style = MaterialTheme.typography.bodySmall,
                            fontFamily = FontFamily.Monospace,
                        )
                    }
                }
            }
            HorizontalDivider()
        }
    }
}

@Composable
private fun IdentityTab(vm: AdminViewModel) {
    val ownId by vm.ownNodeId.collectAsStateWithLifecycle()
    val ctx = LocalContext.current

    var pendingExport by remember { mutableStateOf<String?>(null) }
    val exportLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.CreateDocument("application/toml"),
    ) { uri: Uri? ->
        val blob = pendingExport
        pendingExport = null
        if (uri != null && blob != null) {
            ctx.contentResolver.openOutputStream(uri)?.use { it.write(blob.toByteArray()) }
            Toast.makeText(ctx, "Identity exported", Toast.LENGTH_SHORT).show()
        }
    }
    val importLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.OpenDocument(),
    ) { uri: Uri? ->
        if (uri != null) {
            val text = ctx.contentResolver.openInputStream(uri)?.use {
                it.bufferedReader().readText()
            } ?: return@rememberLauncherForActivityResult
            vm.importIdentityBlob(text) { /* toast handled in VM */ }
        }
    }

    Column(
        modifier = Modifier.fillMaxSize().padding(16.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(stringResource(R.string.my_identity_title), style = MaterialTheme.typography.titleLarge)
        Spacer(Modifier.height(8.dp))
        Text(
            stringResource(R.string.my_identity_blurb),
            style = MaterialTheme.typography.bodySmall,
        )
        Spacer(Modifier.height(16.dp))
        if (ownId.isNotBlank()) {
            val bmp = remember(ownId) { runCatching { QrEncoder.encode(ownId, sizePx = 512) }.getOrNull() }
            bmp?.let {
                Image(
                    bitmap = it.asImageBitmap(),
                    contentDescription = "QR for own NodeId",
                    modifier = Modifier
                        .size(220.dp)
                        .background(androidx.compose.ui.graphics.Color.White)
                        .padding(8.dp),
                )
            }
            Spacer(Modifier.height(8.dp))
            Text(
                ownId,
                style = MaterialTheme.typography.bodySmall,
                fontFamily = FontFamily.Monospace,
                modifier = Modifier.padding(horizontal = 16.dp),
            )
        } else {
            CircularProgressIndicator()
        }
        Spacer(Modifier.height(24.dp))
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            TextButton(onClick = {
                val blob = vm.exportIdentityBlob("herd-scout-admin") ?: return@TextButton
                pendingExport = blob
                exportLauncher.launch("herd-scout-admin-identity.toml")
            }) {
                Text(stringResource(R.string.action_export_identity))
            }
            TextButton(onClick = {
                importLauncher.launch(arrayOf("application/toml", "text/plain", "*/*"))
            }) {
                Text(stringResource(R.string.action_import_identity))
            }
        }
        Spacer(Modifier.height(8.dp))
        Text(
            stringResource(R.string.export_warning_message),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.error,
        )
    }
}

@Composable
private fun stringResource(id: Int): String = androidx.compose.ui.res.stringResource(id)
