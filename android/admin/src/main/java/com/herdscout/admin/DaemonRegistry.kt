package com.herdscout.admin

import android.content.Context
import android.content.SharedPreferences
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.serialization.Serializable
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.json.Json

/**
 * Persistent list of saved daemons (Decision 12 — fleet mode).
 *
 * Up to [MAX_DAEMONS] entries, evicting the LRU on overflow. Stored as
 * a single JSON-encoded list in `SharedPreferences` so we don't fight
 * the platform's tiny-key API for what's effectively one blob.
 */
@Serializable
data class DaemonEntry(
    val nodeId: String,
    val label: String,
    val lastConnectedMs: Long = 0L,
)

class DaemonRegistry(context: Context) {
    private val prefs: SharedPreferences =
        context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

    private val _entries = MutableStateFlow(load())
    val entries: StateFlow<List<DaemonEntry>> = _entries.asStateFlow()

    private val _activeNodeId = MutableStateFlow(prefs.getString(KEY_ACTIVE, null))
    val activeNodeId: StateFlow<String?> = _activeNodeId.asStateFlow()

    fun upsert(entry: DaemonEntry) {
        val current = _entries.value.toMutableList()
        val existingIdx = current.indexOfFirst { it.nodeId == entry.nodeId }
        if (existingIdx >= 0) {
            current[existingIdx] = entry
        } else {
            current.add(0, entry)
            // LRU eviction by lastConnectedMs (oldest goes).
            if (current.size > MAX_DAEMONS) {
                val oldest = current.minByOrNull { it.lastConnectedMs }
                if (oldest != null) current.remove(oldest)
            }
        }
        _entries.value = current
        save(current)
    }

    fun remove(nodeId: String) {
        val current = _entries.value.filterNot { it.nodeId == nodeId }
        _entries.value = current
        save(current)
        if (_activeNodeId.value == nodeId) {
            setActive(null)
        }
    }

    fun setActive(nodeId: String?) {
        _activeNodeId.value = nodeId
        prefs.edit().apply {
            if (nodeId == null) remove(KEY_ACTIVE) else putString(KEY_ACTIVE, nodeId)
        }.apply()
    }

    fun activeEntry(): DaemonEntry? {
        val id = _activeNodeId.value ?: return null
        return _entries.value.firstOrNull { it.nodeId == id }
    }

    fun touchLastConnected(nodeId: String) {
        val current = _entries.value.toMutableList()
        val idx = current.indexOfFirst { it.nodeId == nodeId }
        if (idx >= 0) {
            current[idx] = current[idx].copy(lastConnectedMs = System.currentTimeMillis())
            _entries.value = current
            save(current)
        }
    }

    private fun load(): List<DaemonEntry> {
        val raw = prefs.getString(KEY_ENTRIES, null) ?: return emptyList()
        return try {
            JSON.decodeFromString(ListSerializer(DaemonEntry.serializer()), raw)
        } catch (_: Throwable) {
            emptyList()
        }
    }

    private fun save(list: List<DaemonEntry>) {
        prefs.edit()
            .putString(KEY_ENTRIES, JSON.encodeToString(ListSerializer(DaemonEntry.serializer()), list))
            .apply()
    }

    companion object {
        private const val PREFS_NAME = "herd_scout_admin_daemons"
        private const val KEY_ENTRIES = "entries"
        private const val KEY_ACTIVE = "active"
        const val MAX_DAEMONS = 10
        private val JSON = Json { ignoreUnknownKeys = true }
    }
}
