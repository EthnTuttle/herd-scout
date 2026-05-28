package com.herdscout.admin

import android.app.Application
import android.util.Log
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import androidx.paging.Pager
import androidx.paging.PagingConfig
import androidx.paging.PagingData
import androidx.paging.cachedIn
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Holds the admin app's UI state and orchestrates JNI calls on
 * [Dispatchers.IO]. Single ViewModel for the whole activity since the
 * three tabs share one daemon connection.
 */
class AdminViewModel(application: Application) : AndroidViewModel(application) {

    private val filesDir: String = application.filesDir.absolutePath
    private val registry = DaemonRegistry(application)
    private val audit = AuditDatabase.get(application).auditDao()

    private val _connection = MutableStateFlow(ConnectionState.Disconnected)
    val connection: StateFlow<ConnectionState> = _connection.asStateFlow()

    private val _allowed = MutableStateFlow<List<AllowedEntry>>(emptyList())
    val allowed: StateFlow<List<AllowedEntry>> = _allowed.asStateFlow()

    private val _status = MutableStateFlow<StatusReply?>(null)
    val status: StateFlow<StatusReply?> = _status.asStateFlow()

    private val _ownNodeId = MutableStateFlow("")
    val ownNodeId: StateFlow<String> = _ownNodeId.asStateFlow()

    private val _toast = MutableStateFlow<String?>(null)
    val toast: StateFlow<String?> = _toast.asStateFlow()

    val daemons: StateFlow<List<DaemonEntry>> = registry.entries
    val activeDaemonId: StateFlow<String?> = registry.activeNodeId

    /** Local Room-backed history for the active daemon. */
    @OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)
    val localHistory: Flow<PagingData<AuditEvent>> = registry.activeNodeId
        .flatMapLatest { id ->
            if (id == null) flowOf(PagingData.empty())
            else Pager(
                config = PagingConfig(pageSize = 30, prefetchDistance = 10),
                pagingSourceFactory = { audit.pagingForDaemon(id) },
            ).flow
        }
        .cachedIn(viewModelScope)

    private val _daemonHistory = MutableStateFlow<List<AuditRecord>>(emptyList())
    val daemonHistory: StateFlow<List<AuditRecord>> = _daemonHistory.asStateFlow()

    init {
        viewModelScope.launch(Dispatchers.IO) {
            HerdScoutAdminJni.whoami(filesDir).onSuccess { _ownNodeId.value = it }
        }
        // Auto-reconnect to the previously active daemon on launch.
        viewModelScope.launch {
            registry.activeNodeId.value?.let { reconnect(it) }
        }
    }

    fun addDaemon(label: String, nodeId: String) {
        registry.upsert(DaemonEntry(nodeId = nodeId.trim(), label = label.trim()))
    }

    fun selectDaemon(nodeId: String) {
        viewModelScope.launch { reconnect(nodeId) }
    }

    fun forgetDaemon(nodeId: String) {
        viewModelScope.launch(Dispatchers.IO) {
            HerdScoutAdminJni.disconnect()
        }
        registry.remove(nodeId)
    }

    private suspend fun reconnect(nodeId: String) {
        _connection.value = ConnectionState.Connecting
        val ok = withContext(Dispatchers.IO) {
            HerdScoutAdminJni.disconnect()
            HerdScoutAdminJni.connect(filesDir, nodeId)
        }
        if (ok) {
            registry.setActive(nodeId)
            registry.touchLastConnected(nodeId)
            audit.insert(
                AuditEvent(
                    tsMs = System.currentTimeMillis(),
                    daemonNodeId = nodeId,
                    kind = "connect",
                ),
            )
            _connection.value = ConnectionState.Connected
            refreshAll(nodeId)
        } else {
            _connection.value = ConnectionState.Disconnected
            _toast.value = "Failed to connect to daemon"
        }
    }

    fun refreshAll() {
        val id = registry.activeNodeId.value ?: return
        viewModelScope.launch { refreshAll(id) }
    }

    private suspend fun refreshAll(daemonId: String) {
        withContext(Dispatchers.IO) {
            HerdScoutAdminJni.listAllowed(filesDir, daemonId)
                .onSuccess { _allowed.value = it }
                .onFailure { _toast.value = it.message }
            HerdScoutAdminJni.status(filesDir, daemonId)
                .onSuccess { _status.value = it }
            HerdScoutAdminJni.tailAudit(filesDir, daemonId, lastN = 50, beforeTsMs = null)
                .onSuccess { tail ->
                    _daemonHistory.value = tail.records
                    // Cache as `daemon_replay` rows so the offline view
                    // has something to show.
                    audit.clearDaemonReplay(daemonId)
                    audit.insertAll(
                        tail.records.map { rec ->
                            AuditEvent(
                                tsMs = rec.tsMs,
                                daemonNodeId = daemonId,
                                kind = "daemon_replay",
                                op = rec.kind,
                                targetNodeId = rec.actorNodeId,
                            )
                        },
                    )
                }
        }
    }

    fun addAllowed(targetNodeId: String, label: String) {
        val daemonId = registry.activeNodeId.value ?: return
        viewModelScope.launch(Dispatchers.IO) {
            audit.insert(
                AuditEvent(
                    tsMs = System.currentTimeMillis(),
                    daemonNodeId = daemonId,
                    kind = "rpc_attempt",
                    op = "add_allowed",
                    targetNodeId = targetNodeId,
                    targetLabel = label,
                ),
            )
            HerdScoutAdminJni.addAllowed(filesDir, daemonId, targetNodeId, label)
                .onSuccess {
                    audit.insert(
                        AuditEvent(
                            tsMs = System.currentTimeMillis(),
                            daemonNodeId = daemonId,
                            kind = "rpc_success",
                            op = "add_allowed",
                            targetNodeId = targetNodeId,
                            targetLabel = label,
                        ),
                    )
                    refreshAll(daemonId)
                }
                .onFailure { e ->
                    val err = e as? AdminError
                    audit.insert(
                        AuditEvent(
                            tsMs = System.currentTimeMillis(),
                            daemonNodeId = daemonId,
                            kind = "rpc_error",
                            op = "add_allowed",
                            targetNodeId = targetNodeId,
                            targetLabel = label,
                            errorCode = err?.code,
                            errorMessage = e.message,
                        ),
                    )
                    _toast.value = e.message
                }
        }
    }

    fun removeAllowed(targetNodeId: String) {
        val daemonId = registry.activeNodeId.value ?: return
        viewModelScope.launch(Dispatchers.IO) {
            audit.insert(
                AuditEvent(
                    tsMs = System.currentTimeMillis(),
                    daemonNodeId = daemonId,
                    kind = "rpc_attempt",
                    op = "remove_allowed",
                    targetNodeId = targetNodeId,
                ),
            )
            HerdScoutAdminJni.removeAllowed(filesDir, daemonId, targetNodeId)
                .onSuccess {
                    audit.insert(
                        AuditEvent(
                            tsMs = System.currentTimeMillis(),
                            daemonNodeId = daemonId,
                            kind = "rpc_success",
                            op = "remove_allowed",
                            targetNodeId = targetNodeId,
                        ),
                    )
                    refreshAll(daemonId)
                }
                .onFailure { e ->
                    val err = e as? AdminError
                    audit.insert(
                        AuditEvent(
                            tsMs = System.currentTimeMillis(),
                            daemonNodeId = daemonId,
                            kind = "rpc_error",
                            op = "remove_allowed",
                            targetNodeId = targetNodeId,
                            errorCode = err?.code,
                            errorMessage = e.message,
                        ),
                    )
                    _toast.value = e.message
                }
        }
    }

    fun exportIdentityBlob(label: String): String? {
        return HerdScoutAdminJni.exportIdentity(filesDir, label).getOrElse {
            Log.w(TAG, "export failed: ${it.message}")
            _toast.value = it.message
            null
        }
    }

    fun importIdentityBlob(envelope: String, onDone: (String?) -> Unit) {
        viewModelScope.launch(Dispatchers.IO) {
            HerdScoutAdminJni.importIdentity(filesDir, envelope)
                .onSuccess { newId ->
                    _ownNodeId.value = newId
                    _connection.value = ConnectionState.Disconnected
                    _toast.value = "Identity imported. Reconnect to a daemon to continue."
                    onDone(newId)
                }
                .onFailure {
                    _toast.value = it.message
                    onDone(null)
                }
        }
    }

    fun consumeToast() {
        _toast.value = null
    }

    enum class ConnectionState {
        Disconnected,
        Connecting,
        Connected,
    }

    companion object {
        private const val TAG = "AdminViewModel"
    }
}
