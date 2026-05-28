package com.herdscout.admin

import com.herdscout.shared.HerdScoutJniLoader
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject

/**
 * Kotlin facade over the Wave 12 admin JNI surface.
 *
 * Every native call returns a JSON string we parse here; the Rust
 * side serializes via `serde_json` and Kotlin parses with
 * `kotlinx.serialization`. The wire format is intentionally string-
 * based so the Rust enum can grow new variants without regenerating
 * Kotlin bindings.
 *
 * All native methods are blocking — call them from `Dispatchers.IO`.
 */
object HerdScoutAdminJni {
    init {
        HerdScoutJniLoader.ensureLoaded()
    }

    private val json = Json {
        ignoreUnknownKeys = true
        coerceInputValues = true
    }

    // ── Identity ────────────────────────────────────────────────────────

    fun whoami(filesDir: String): Result<String> {
        val s = nativeIdentityWhoami(filesDir)
        return parseJsonOrPlain(s) { plain -> plain }
    }

    fun exportIdentity(filesDir: String, label: String): Result<String> {
        val s = nativeIdentityExport(filesDir, label)
        // Export returns the raw envelope blob (TOML), not JSON. If
        // the Rust side errored it returns a JSON error object that
        // happens to start with `{"type":"error"`.
        return if (s.startsWith("{\"type\":\"error\"")) {
            parseError(s)
        } else {
            Result.success(s)
        }
    }

    fun importIdentity(filesDir: String, envelope: String): Result<String> {
        val s = nativeIdentityImport(filesDir, envelope)
        return parseOkObject(s) { obj ->
            (obj["node_id"] as? kotlinx.serialization.json.JsonPrimitive)
                ?.content
                ?: error("missing node_id in import reply")
        }
    }

    // ── Session lifecycle ───────────────────────────────────────────────

    fun connect(filesDir: String, daemonNodeId: String): Boolean =
        nativeAdminConnect(filesDir, daemonNodeId) != 0L

    fun disconnect(): Boolean = nativeAdminDisconnect()

    // ── RPCs ────────────────────────────────────────────────────────────

    fun listAllowed(filesDir: String, daemonNodeId: String): Result<List<AllowedEntry>> {
        val s = nativeAdminListAllowed(filesDir, daemonNodeId)
        return parseOkObject(s) { obj ->
            json.decodeFromJsonElement(
                kotlinx.serialization.builtins.ListSerializer(AllowedEntry.serializer()),
                obj["entries"] ?: error("missing entries"),
            )
        }
    }

    fun addAllowed(
        filesDir: String,
        daemonNodeId: String,
        targetNodeId: String,
        label: String,
    ): Result<Unit> {
        val s = nativeAdminAddAllowed(filesDir, daemonNodeId, targetNodeId, label)
        return parseOk(s)
    }

    fun removeAllowed(
        filesDir: String,
        daemonNodeId: String,
        targetNodeId: String,
    ): Result<Unit> {
        val s = nativeAdminRemoveAllowed(filesDir, daemonNodeId, targetNodeId)
        return parseOk(s)
    }

    fun status(filesDir: String, daemonNodeId: String): Result<StatusReply> {
        val s = nativeAdminStatus(filesDir, daemonNodeId)
        return parseOkObject(s) { obj ->
            json.decodeFromJsonElement(StatusReply.serializer(), obj["status"] ?: error("missing status"))
        }
    }

    fun tailAudit(
        filesDir: String,
        daemonNodeId: String,
        lastN: Int,
        beforeTsMs: Long?,
    ): Result<AuditTail> {
        val before = beforeTsMs ?: -1L
        val s = nativeAdminTailAudit(filesDir, daemonNodeId, lastN, before)
        return parseOkObject(s) { obj ->
            AuditTail(
                records = json.decodeFromJsonElement(
                    kotlinx.serialization.builtins.ListSerializer(AuditRecord.serializer()),
                    obj["records"] ?: error("missing records"),
                ),
                eof = (obj["eof"] as? kotlinx.serialization.json.JsonPrimitive)?.content?.toBoolean() ?: false,
            )
        }
    }

    // ── Reply parsing helpers ───────────────────────────────────────────

    private fun parseError(s: String): Result<Nothing> {
        return try {
            val obj = json.parseToJsonElement(s).let { it as? JsonObject } ?: throw IllegalStateException("not json")
            val code = (obj["code"] as? kotlinx.serialization.json.JsonPrimitive)?.content ?: "unknown"
            val message = (obj["message"] as? kotlinx.serialization.json.JsonPrimitive)?.content ?: s
            Result.failure(AdminError(code, message))
        } catch (e: Throwable) {
            Result.failure(AdminError("parse", "Failed to parse error reply: $s"))
        }
    }

    private fun parseOk(s: String): Result<Unit> {
        return try {
            val obj = json.parseToJsonElement(s) as? JsonObject ?: return parseError(s)
            val type = (obj["type"] as? kotlinx.serialization.json.JsonPrimitive)?.content
            if (type == "ok") Result.success(Unit) else parseError(s)
        } catch (e: Throwable) {
            Result.failure(AdminError("parse", e.message ?: "parse error"))
        }
    }

    private fun <T> parseOkObject(s: String, extract: (JsonObject) -> T): Result<T> {
        return try {
            val obj = json.parseToJsonElement(s) as? JsonObject ?: return parseError(s)
            val type = (obj["type"] as? kotlinx.serialization.json.JsonPrimitive)?.content
            if (type == "ok") Result.success(extract(obj)) else parseError(s)
        } catch (e: Throwable) {
            Result.failure(AdminError("parse", e.message ?: "parse error"))
        }
    }

    /**
     * For the `whoami` path, which returns a bare NodeId string when
     * happy, or a JSON error object when sad.
     */
    private fun parseJsonOrPlain(s: String, ifPlain: (String) -> String): Result<String> {
        return if (s.startsWith("{\"type\":\"error\"")) {
            parseError(s)
        } else {
            Result.success(ifPlain(s))
        }
    }

    // ── External native exports ─────────────────────────────────────────

    private external fun nativeIdentityWhoami(filesDir: String): String
    private external fun nativeIdentityExport(filesDir: String, label: String): String
    private external fun nativeIdentityImport(filesDir: String, envelope: String): String

    private external fun nativeAdminConnect(filesDir: String, daemonNodeId: String): Long
    private external fun nativeAdminDisconnect(): Boolean

    private external fun nativeAdminListAllowed(filesDir: String, daemonNodeId: String): String
    private external fun nativeAdminAddAllowed(
        filesDir: String,
        daemonNodeId: String,
        targetNodeId: String,
        label: String,
    ): String
    private external fun nativeAdminRemoveAllowed(
        filesDir: String,
        daemonNodeId: String,
        targetNodeId: String,
    ): String
    private external fun nativeAdminStatus(filesDir: String, daemonNodeId: String): String
    private external fun nativeAdminTailAudit(
        filesDir: String,
        daemonNodeId: String,
        lastN: Int,
        beforeTsMs: Long,
    ): String
}

/** Daemon-reported error. `code` matches the strings the daemon returns. */
class AdminError(val code: String, override val message: String) : Throwable(message)

@Serializable
data class AllowedEntry(
    @SerialName("node_id") val nodeId: String,
    val label: String = "",
)

@Serializable
data class StatusReply(
    @SerialName("daemon_version") val daemonVersion: String,
    @SerialName("own_node_id") val ownNodeId: String,
    @SerialName("active_ssh_sessions") val activeSshSessions: Int,
    @SerialName("admins_count") val adminsCount: Int,
    @SerialName("allowed_count") val allowedCount: Int,
    @SerialName("last_reload_unix_ms") val lastReloadUnixMs: Long,
    @SerialName("last_reload_source") val lastReloadSource: String,
    @SerialName("identity_schema_version") val identitySchemaVersion: Int,
)

@Serializable
data class AuditRecord(
    @SerialName("schema_version") val schemaVersion: Int,
    @SerialName("ts_ms") val tsMs: Long,
    val kind: String,
    @SerialName("actor_node_id") val actorNodeId: String? = null,
    @SerialName("actor_label") val actorLabel: String? = null,
    val details: kotlinx.serialization.json.JsonElement = kotlinx.serialization.json.JsonObject(emptyMap()),
)

data class AuditTail(
    val records: List<AuditRecord>,
    val eof: Boolean,
)
