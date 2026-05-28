package com.herdscout.admin

import android.content.Context
import androidx.paging.PagingSource
import androidx.room.Dao
import androidx.room.Database
import androidx.room.Entity
import androidx.room.Insert
import androidx.room.PrimaryKey
import androidx.room.Query
import androidx.room.Room
import androidx.room.RoomDatabase

/**
 * Wave 12 Decision 9 — phone-side audit log.
 *
 * Records every RPC the user initiates from this device, including
 * attempts that never reached the daemon (network down, daemon
 * refused us). Complements the daemon-side log; the two diverge over
 * time on purpose.
 *
 * `kind` strings:
 *   - `rpc_attempt` — written before a mutating call leaves the device
 *   - `rpc_success` — daemon returned Ok
 *   - `rpc_error`   — daemon returned an error or transport failed
 *   - `connect`     — successful connect_session
 *   - `disconnect`  — successful disconnect_session
 *   - `daemon_replay` — cached row from a successful TailAudit (offline view)
 */
@Entity(tableName = "audit_events")
data class AuditEvent(
    @PrimaryKey(autoGenerate = true) val id: Long = 0,
    val tsMs: Long,
    val daemonNodeId: String,
    val kind: String,
    val op: String? = null,
    val targetNodeId: String? = null,
    val targetLabel: String? = null,
    val errorCode: String? = null,
    val errorMessage: String? = null,
    val schemaVersion: Int = 1,
)

@Dao
interface AuditDao {
    @Insert
    suspend fun insert(event: AuditEvent): Long

    @Insert
    suspend fun insertAll(events: List<AuditEvent>)

    @Query(
        "SELECT * FROM audit_events WHERE daemonNodeId = :daemonNodeId ORDER BY tsMs DESC, id DESC"
    )
    fun pagingForDaemon(daemonNodeId: String): PagingSource<Int, AuditEvent>

    @Query(
        "DELETE FROM audit_events WHERE daemonNodeId = :daemonNodeId AND kind = 'daemon_replay'"
    )
    suspend fun clearDaemonReplay(daemonNodeId: String)

    @Query("DELETE FROM audit_events")
    suspend fun clearAll()
}

@Database(entities = [AuditEvent::class], version = 1, exportSchema = false)
abstract class AuditDatabase : RoomDatabase() {
    abstract fun auditDao(): AuditDao

    companion object {
        @Volatile
        private var INSTANCE: AuditDatabase? = null

        fun get(context: Context): AuditDatabase =
            INSTANCE ?: synchronized(this) {
                INSTANCE ?: Room.databaseBuilder(
                    context.applicationContext,
                    AuditDatabase::class.java,
                    "herd_scout_admin_audit.db",
                )
                    .fallbackToDestructiveMigration()
                    .build()
                    .also { INSTANCE = it }
            }
    }
}
