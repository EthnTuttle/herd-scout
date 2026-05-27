//! herd-scout-jni — Android JNI bridge.
//!
//! Two surfaces:
//!   - `streaming` (Android-only): the existing camera publisher used by
//!     `com.herdscout.app`. Captures CameraX frames, encodes via
//!     MediaCodec, publishes a moq broadcast through `iroh-live`.
//!   - `admin_client` (always-on): Wave 12 admin RPC client used by
//!     `com.herdscout.admin`. Speaks the `herd-scout/admin/1` ALPN
//!     against a daemon NodeId. Builds + tests on host so unit
//!     coverage doesn't require the NDK.
//!
//! Android JNI exports for both live in this file, gated `cfg(target_os
//! = "android")`. Host builds expose only the `admin_client` module.

pub mod admin_client;

#[cfg(target_os = "android")]
mod streaming;

// ── Android-only admin JNI exports ──────────────────────────────────────

#[cfg(target_os = "android")]
mod admin_jni {
    use std::path::PathBuf;
    use std::sync::OnceLock;

    use jni::JNIEnv;
    use jni::objects::{JClass, JString};
    use jni::sys::{jboolean, jlong};
    use serde::Serialize;
    use serde_json::json;
    use tokio::runtime::Runtime;
    use tracing::error;

    use crate::admin_client;

    /// Tokio runtime shared by the admin JNI exports. Distinct from
    /// the streaming-side runtime so admin RPC latency doesn't compete
    /// with camera-frame delivery.
    static ADMIN_RUNTIME: OnceLock<Runtime> = OnceLock::new();

    fn runtime() -> &'static Runtime {
        ADMIN_RUNTIME.get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_name("herd-scout-admin")
                .build()
                .expect("admin tokio runtime")
        })
    }

    fn read_jstring(env: &mut JNIEnv<'_>, s: &JString<'_>) -> Option<String> {
        match env.get_string(s) {
            Ok(s) => Some(s.into()),
            Err(e) => {
                error!("admin_jni: failed to read JString: {e}");
                None
            }
        }
    }

    fn new_jstring<'a>(env: &mut JNIEnv<'a>, s: &str) -> JString<'a> {
        env.new_string(s).unwrap_or_else(|_| {
            env.new_string("").expect("empty new_string fallback")
        })
    }

    /// Helper: serialize a value to JSON and return as a JString. On
    /// failure returns an empty string (Kotlin parses that as a JSON
    /// error and surfaces it).
    fn json_jstring<'a, T: Serialize>(env: &mut JNIEnv<'a>, v: &T) -> JString<'a> {
        match serde_json::to_string(v) {
            Ok(s) => new_jstring(env, &s),
            Err(e) => {
                let fallback = format!(
                    "{{\"type\":\"error\",\"code\":\"serialize\",\"message\":\"{e}\"}}"
                );
                new_jstring(env, &fallback)
            }
        }
    }

    fn error_json<'a>(env: &mut JNIEnv<'a>, code: &str, message: &str) -> JString<'a> {
        let v = json!({ "type": "error", "code": code, "message": message });
        new_jstring(env, &v.to_string())
    }

    /// Given a Kotlin-supplied `filesDir` JString, parse into a PathBuf.
    fn parse_files_dir(env: &mut JNIEnv<'_>, s: &JString<'_>) -> Option<PathBuf> {
        read_jstring(env, s).map(PathBuf::from)
    }

    // ── Identity ────────────────────────────────────────────────────────

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_herdscout_admin_HerdScoutAdminJni_nativeIdentityWhoami<'a>(
        mut env: JNIEnv<'a>,
        _class: JClass<'_>,
        files_dir: JString<'_>,
    ) -> JString<'a> {
        let Some(dir) = parse_files_dir(&mut env, &files_dir) else {
            return error_json(&mut env, "bad_args", "filesDir not readable");
        };
        match admin_client::whoami(&dir) {
            Ok(s) => new_jstring(&mut env, &s),
            Err(e) => error_json(&mut env, "identity", &format!("{e:#}")),
        }
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_herdscout_admin_HerdScoutAdminJni_nativeIdentityExport<'a>(
        mut env: JNIEnv<'a>,
        _class: JClass<'_>,
        files_dir: JString<'_>,
        label: JString<'_>,
    ) -> JString<'a> {
        let (Some(dir), Some(label)) = (
            parse_files_dir(&mut env, &files_dir),
            read_jstring(&mut env, &label),
        ) else {
            return error_json(&mut env, "bad_args", "filesDir/label not readable");
        };
        match admin_client::identity_export(&dir, &label) {
            Ok(s) => new_jstring(&mut env, &s),
            Err(e) => error_json(&mut env, "identity", &format!("{e:#}")),
        }
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_herdscout_admin_HerdScoutAdminJni_nativeIdentityImport<'a>(
        mut env: JNIEnv<'a>,
        _class: JClass<'_>,
        files_dir: JString<'_>,
        envelope: JString<'_>,
    ) -> JString<'a> {
        let (Some(dir), Some(envelope)) = (
            parse_files_dir(&mut env, &files_dir),
            read_jstring(&mut env, &envelope),
        ) else {
            return error_json(&mut env, "bad_args", "filesDir/envelope not readable");
        };
        match runtime().block_on(admin_client::identity_import(&dir, &envelope)) {
            Ok(node_id) => new_jstring(
                &mut env,
                &json!({ "type": "ok", "node_id": node_id }).to_string(),
            ),
            Err(e) => error_json(&mut env, "identity_import", &format!("{e:#}")),
        }
    }

    // ── Session lifecycle ───────────────────────────────────────────────

    /// Connect (or reuse the existing connection if it points at the
    /// same daemon). Returns 1 on success, 0 on failure. The actual
    /// session lives in `admin_client::ADMIN_SESSION`; the Kotlin
    /// layer never holds a raw handle.
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_herdscout_admin_HerdScoutAdminJni_nativeAdminConnect(
        mut env: JNIEnv<'_>,
        _class: JClass<'_>,
        files_dir: JString<'_>,
        daemon_node_id: JString<'_>,
    ) -> jlong {
        let (Some(dir), Some(daemon)) = (
            parse_files_dir(&mut env, &files_dir),
            read_jstring(&mut env, &daemon_node_id),
        ) else {
            error!("admin_jni: connect missing args");
            return 0;
        };
        match runtime().block_on(admin_client::connect_session(&dir, &daemon)) {
            Ok(_session) => 1,
            Err(e) => {
                error!("admin_jni: connect failed: {e:#}");
                0
            }
        }
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_herdscout_admin_HerdScoutAdminJni_nativeAdminDisconnect(
        _env: JNIEnv<'_>,
        _class: JClass<'_>,
    ) -> jboolean {
        let was_open = runtime().block_on(admin_client::disconnect_session());
        if was_open {
            jni::sys::JNI_TRUE
        } else {
            jni::sys::JNI_FALSE
        }
    }

    // ── Admin RPCs ──────────────────────────────────────────────────────
    //
    // Each takes a `daemon_node_id` JString so we can transparently
    // reuse-or-reconnect the single-slot session if the Kotlin side
    // forgot to call connect first (or the user switched daemons).

    async fn session_for(
        files_dir: &std::path::Path,
        daemon: &str,
    ) -> anyhow::Result<std::sync::Arc<admin_client::AdminSession>> {
        admin_client::connect_session(files_dir, daemon).await
    }

    macro_rules! rpc_jni {
        ($name:ident, $handler:expr) => {
            $handler
        };
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_herdscout_admin_HerdScoutAdminJni_nativeAdminListAllowed<'a>(
        mut env: JNIEnv<'a>,
        _class: JClass<'_>,
        files_dir: JString<'_>,
        daemon_node_id: JString<'_>,
    ) -> JString<'a> {
        let (Some(dir), Some(daemon)) = (
            parse_files_dir(&mut env, &files_dir),
            read_jstring(&mut env, &daemon_node_id),
        ) else {
            return error_json(&mut env, "bad_args", "filesDir/daemon_node_id not readable");
        };
        let res = runtime().block_on(async {
            let s = session_for(&dir, &daemon).await?;
            s.list_allowed().await
        });
        match res {
            Ok(entries) => {
                let v = json!({ "type": "ok", "entries": entries });
                new_jstring(&mut env, &v.to_string())
            }
            Err(e) => error_json(&mut env, "rpc", &format!("{e:#}")),
        }
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_herdscout_admin_HerdScoutAdminJni_nativeAdminAddAllowed<'a>(
        mut env: JNIEnv<'a>,
        _class: JClass<'_>,
        files_dir: JString<'_>,
        daemon_node_id: JString<'_>,
        target_node_id: JString<'_>,
        label: JString<'_>,
    ) -> JString<'a> {
        let (Some(dir), Some(daemon), Some(target), Some(label)) = (
            parse_files_dir(&mut env, &files_dir),
            read_jstring(&mut env, &daemon_node_id),
            read_jstring(&mut env, &target_node_id),
            read_jstring(&mut env, &label),
        ) else {
            return error_json(&mut env, "bad_args", "args not readable");
        };
        let res = runtime().block_on(async {
            let s = session_for(&dir, &daemon).await?;
            s.add_allowed(&target, &label).await
        });
        match res {
            Ok(()) => new_jstring(&mut env, &json!({ "type": "ok" }).to_string()),
            Err(e) => error_json(&mut env, "rpc", &format!("{e:#}")),
        }
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_herdscout_admin_HerdScoutAdminJni_nativeAdminRemoveAllowed<'a>(
        mut env: JNIEnv<'a>,
        _class: JClass<'_>,
        files_dir: JString<'_>,
        daemon_node_id: JString<'_>,
        target_node_id: JString<'_>,
    ) -> JString<'a> {
        let (Some(dir), Some(daemon), Some(target)) = (
            parse_files_dir(&mut env, &files_dir),
            read_jstring(&mut env, &daemon_node_id),
            read_jstring(&mut env, &target_node_id),
        ) else {
            return error_json(&mut env, "bad_args", "args not readable");
        };
        let res = runtime().block_on(async {
            let s = session_for(&dir, &daemon).await?;
            s.remove_allowed(&target).await
        });
        match res {
            Ok(()) => new_jstring(&mut env, &json!({ "type": "ok" }).to_string()),
            Err(e) => error_json(&mut env, "rpc", &format!("{e:#}")),
        }
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_herdscout_admin_HerdScoutAdminJni_nativeAdminStatus<'a>(
        mut env: JNIEnv<'a>,
        _class: JClass<'_>,
        files_dir: JString<'_>,
        daemon_node_id: JString<'_>,
    ) -> JString<'a> {
        let (Some(dir), Some(daemon)) = (
            parse_files_dir(&mut env, &files_dir),
            read_jstring(&mut env, &daemon_node_id),
        ) else {
            return error_json(&mut env, "bad_args", "args not readable");
        };
        let res = runtime().block_on(async {
            let s = session_for(&dir, &daemon).await?;
            s.status().await
        });
        match res {
            Ok(status) => json_jstring(&mut env, &json!({ "type": "ok", "status": status })),
            Err(e) => error_json(&mut env, "rpc", &format!("{e:#}")),
        }
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_herdscout_admin_HerdScoutAdminJni_nativeAdminTailAudit<'a>(
        mut env: JNIEnv<'a>,
        _class: JClass<'_>,
        files_dir: JString<'_>,
        daemon_node_id: JString<'_>,
        last_n: jni::sys::jint,
        before_ts_ms: jlong, // -1 means "no filter"; Kotlin uses -1 instead of nullable Long
    ) -> JString<'a> {
        let (Some(dir), Some(daemon)) = (
            parse_files_dir(&mut env, &files_dir),
            read_jstring(&mut env, &daemon_node_id),
        ) else {
            return error_json(&mut env, "bad_args", "args not readable");
        };
        let n = last_n.max(0) as u32;
        let before = if before_ts_ms < 0 {
            None
        } else {
            Some(before_ts_ms as u64)
        };
        let res = runtime().block_on(async {
            let s = session_for(&dir, &daemon).await?;
            s.tail_audit(n, before).await
        });
        match res {
            Ok((records, eof)) => json_jstring(
                &mut env,
                &json!({ "type": "ok", "records": records, "eof": eof }),
            ),
            Err(e) => error_json(&mut env, "rpc", &format!("{e:#}")),
        }
    }
}

#[cfg(target_os = "android")]
pub use admin_jni::*;
