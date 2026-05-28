# herd-scout-identity

Versioned iroh-identity envelope shared by `herd-scout-daemon`,
`herdctl`, and `herd-scout-jni`. Replaces the ad-hoc 32-raw-byte and
64-hex-char on-disk formats from before Wave 12.

## File format

```toml
schema_version = 1

[identity]
secret_key  = "ab12...cd34"          # 64 lowercase-hex chars (32 bytes)
node_id     = "f9ed1a539ead...409e"  # canonical EndpointId; integrity gate
created_at  = "2026-05-27T10:30:00Z"  # RFC 3339, informational
label       = "Gary's Pixel"

[origin]
device      = "android"               # android | linux | macos | unknown
app_version = "0.1.0"                 # CARGO_PKG_VERSION at write time
```

## Invariants enforced on `load()`

1. `schema_version <= SCHEMA_VERSION` of the current build. A newer
   file returns `IdentityError::UnsupportedSchema { found, max_supported }`
   and the caller refuses to start. Recovery: install a newer build.
2. `secret_key` decodes to exactly 32 bytes (`BadKeyLength` /
   `BadKeyEncoding`).
3. `node_id` matches `SecretKey::public()` of the decoded secret
   (`IntegrityCheckFailed`). A tampered file is loud, not silent —
   we refuse rather than producing a different NodeId than the user
   expects.

## Atomic writes

`save()` and `load_or_generate()` write `<path>.tmp` with mode `0600`,
`fsync`, then `rename` over the target. A crash mid-write leaves the
prior file untouched. `<path>.tmp` from a crashed prior write is
removed best-effort on the next save.

## Legacy migration

`load_or_generate(path, label)` checks two well-known legacy files in
the same parent dir before generating a fresh identity:

| Legacy file       | Format        | Source            |
| ----------------- | ------------- | ----------------- |
| `secret.key`      | 32 raw bytes  | `herdctl` pre-12  |
| `iroh_secret`     | 64 lowercase-hex (+ optional newline) | `herd-scout-daemon` pre-12 |

When found, the file is wrapped in a v1 envelope, written atomically
to the requested path, then **the legacy file is removed only after
the new file is durable on disk**. The daemon's pre-12 file lives in
`<data_dir>` while the v1 envelope lives in `<config_dir>` — that
cross-dir migration is wired explicitly in `herd-scout-daemon/src/daemon_secret.rs`.

## Bumping `SCHEMA_VERSION`

When you add fields:

- **Optional, default-able** — add to `Envelope*` structs with
  `#[serde(default)]`. Old files keep parsing. *Don't* bump the
  version constant.
- **Required for new behavior** — bump `SCHEMA_VERSION`, write the
  new shape always, support the old shape via an "in-memory upgrade"
  path inside `parse_envelope`. Old daemons reading new files refuse
  cleanly via `UnsupportedSchema`.

The pattern is the "schema version per entity, lazy migration, no
flag day" rule from the project wiki's
`[[iroh-docs-fms-schema]]` § Schema evolution.

## Threat model

The envelope file *is* the secret. Treat it like an SSH private key.
We do not encrypt it — a passphrase the operator forgets is worse
than an unencrypted file in a private directory. Future improvement:
optional age-format encryption with a passphrase. v1 ships
unencrypted with explicit UI warnings on the export path.
