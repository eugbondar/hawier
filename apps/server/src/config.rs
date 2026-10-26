//! Server configuration from `HAWIER_*` environment variables.

use std::net::SocketAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Full deployment: Mgmt + DDI + background engines (rollout, auto-assign,
    /// housekeeping). Also serves DDI so a single node is a complete install.
    Control,
    /// DDI + artifact serving + read-only Mgmt; no background loops. Keeps
    /// devices updating when the control node is down.
    Sub,
}

/// A statically configured Mgmt API user (hawkBit keeps users in config too;
/// there is no user table in the schema).
#[derive(Debug, Clone)]
pub struct MgmtUser {
    pub username: String,
    pub password: String,
    pub tenant: String,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub bind_addr: SocketAddr,
    pub database_url: String,
    pub role: Role,
    /// Local artifact directory. Also the upload spool for the S3 backend:
    /// an artifact is named by its own sha1, so it must be written and
    /// hashed somewhere before it can be placed.
    pub artifact_dir: String,
    /// Path to the storage extension's `.wasm` module. Empty means no
    /// storage: the node answers 503 for artifacts rather than inventing a
    /// fallback.
    pub ext_storage: String,
    /// Path to the event-sink extension's `.wasm` module. Empty means events
    /// pile up in the outbox undelivered — visible, not lost.
    pub ext_event_sink: String,
    /// Path to the retention extension's `.wasm` module. Empty means
    /// retention rules never fire — they are recorded and do nothing.
    pub ext_retention: String,
    /// Path to the rollout strategy's `.wasm` module. Empty means rollouts
    /// cannot be divided at all — a hardened node that was given no strategy
    /// refuses to plan one rather than falling back to hawkBit's.
    pub ext_rollout: String,
    /// Path to the console-surfaces extension's `.wasm` module. Empty means
    /// the console shows only what is compiled into it.
    pub ext_console: String,
    /// Path to the authorisation extension's `.wasm` module. Empty means
    /// every authenticated request is allowed — a node that was never told
    /// how to decide has not been told to refuse either.
    pub ext_authz: String,
    /// Where extension parameters are read from and written to, or empty for
    /// "no file": the console then lists what the environment provides and
    /// says which variable turns editing on.
    ///
    /// Node-scoped on purpose. A sub node legitimately needs a different
    /// artifact root than the control node, so this cannot live in the
    /// database every node shares.
    pub ext_params_file: String,
    /// How long any single extension call may take before the child is
    /// killed and the point's failure applies.
    pub ext_deadline_ms: u64,
    /// `user:password:tenant` triples, comma-separated.
    pub mgmt_users: Vec<MgmtUser>,
    /// Bearer token protecting the internal `/ui/v1` state API.
    pub internal_token: String,
    /// Default for tenant config `authentication.targettoken.enabled`
    /// when the tenant has not set it explicitly.
    pub ddi_targettoken_default: bool,
    /// Default for `authentication.gatewaytoken.enabled`.
    pub ddi_gatewaytoken_default: bool,
    /// Accept unauthenticated DDI requests (dev only).
    pub ddi_anonymous: bool,
    /// How long to keep trying the database at startup before giving up.
    pub db_wait_secs: u64,
    pub mgmt_pool_size: u32,
    pub ddi_pool_size: u32,
    /// Global in-flight request cap.
    pub max_connections: usize,
    /// Request-body cap for artifact uploads (bytes; hawkBit defaults to 1 GiB).
    pub max_artifact_size: usize,
    /// Tenant-config cache TTL in ms (0 disables; tests use 0).
    pub tenant_cache_ttl_ms: u64,
    /// Min interval between `last_target_query` writes per target (ms).
    pub poll_touch_interval_ms: i64,
    /// Housekeeping tick interval (action cleanup + session sweep) in ms.
    pub housekeeping_interval_ms: u64,
    /// Node id baked into incident numbers (0..=1023).
    pub node_id: u16,
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn env_bool(key: &str, default: bool) -> bool {
    match std::env::var(key) {
        Ok(v) => matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes"),
        Err(_) => default,
    }
}

fn parse_mgmt_users(raw: &str) -> anyhow::Result<Vec<MgmtUser>> {
    let mut users = Vec::new();
    for entry in raw.split(',').filter(|e| !e.trim().is_empty()) {
        let mut parts = entry.trim().splitn(3, ':');
        let (Some(username), Some(password)) = (parts.next(), parts.next()) else {
            anyhow::bail!("HAWIER_MGMT_USERS entry '{entry}' must be user:password[:tenant]");
        };
        users.push(MgmtUser {
            username: username.to_string(),
            password: password.to_string(),
            tenant: parts.next().unwrap_or("DEFAULT").to_string(),
        });
    }
    anyhow::ensure!(
        !users.is_empty(),
        "HAWIER_MGMT_USERS must define at least one user"
    );
    Ok(users)
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let bind_addr = env_or("HAWIER_BIND_ADDR", "0.0.0.0:8080").parse()?;
        let database_url = env_or(
            "HAWIER_DATABASE_URL",
            "postgres://hawkbit:hawkbit@localhost:5432/hawkbit",
        );
        let role = match std::env::var("HAWIER_ROLE").as_deref() {
            Ok("sub") => Role::Sub,
            _ => Role::Control,
        };
        Ok(Self {
            bind_addr,
            database_url,
            role,
            artifact_dir: env_or("HAWIER_ARTIFACT_DIR", "./artifacts"),
            ext_storage: env_or("HAWIER_EXT_STORAGE", ""),
            ext_event_sink: env_or("HAWIER_EXT_EVENT_SINK", ""),
            ext_retention: env_or("HAWIER_EXT_RETENTION", ""),
            ext_rollout: env_or("HAWIER_EXT_ROLLOUT", ""),
            ext_console: env_or("HAWIER_EXT_CONSOLE", ""),
            ext_authz: env_or("HAWIER_EXT_AUTHZ", ""),
            ext_params_file: env_or("HAWIER_EXT_PARAMS_FILE", ""),
            ext_deadline_ms: env_or("HAWIER_EXT_DEADLINE_MS", "2000").parse()?,
            mgmt_users: parse_mgmt_users(&env_or("HAWIER_MGMT_USERS", "admin:admin:DEFAULT"))?,
            internal_token: env_or("HAWIER_INTERNAL_TOKEN", "dev-internal-token"),
            ddi_targettoken_default: env_bool("HAWIER_DDI_TARGETTOKEN_ENABLED", true),
            ddi_gatewaytoken_default: env_bool("HAWIER_DDI_GATEWAYTOKEN_ENABLED", false),
            ddi_anonymous: env_bool("HAWIER_DDI_ANONYMOUS", false),
            db_wait_secs: env_or("HAWIER_DB_WAIT_SECS", "60").parse()?,
            mgmt_pool_size: env_or("HAWIER_MGMT_POOL_SIZE", "10").parse()?,
            ddi_pool_size: env_or("HAWIER_DDI_POOL_SIZE", "20").parse()?,
            max_connections: env_or("HAWIER_MAX_CONNECTIONS", "4096").parse()?,
            max_artifact_size: env_or("HAWIER_MAX_ARTIFACT_SIZE", "1073741824").parse()?,
            tenant_cache_ttl_ms: env_or("HAWIER_TENANT_CACHE_TTL_MS", "5000").parse()?,
            poll_touch_interval_ms: env_or("HAWIER_POLL_TOUCH_INTERVAL_MS", "30000").parse()?,
            housekeeping_interval_ms: env_or("HAWIER_HOUSEKEEPING_INTERVAL_MS", "60000").parse()?,
            node_id: match std::env::var("HAWIER_NODE_ID") {
                Ok(raw) => {
                    let node: u16 = raw.parse()?;
                    anyhow::ensure!(
                        node <= crate::incident::MAX_NODE_ID,
                        "HAWIER_NODE_ID must be 0..=1023"
                    );
                    node
                }
                Err(_) => crate::incident::default_node_id(),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_users() {
        let users = parse_mgmt_users("admin:admin:DEFAULT, bob:secret").unwrap();
        assert_eq!(users.len(), 2);
        assert_eq!(users[1].username, "bob");
        assert_eq!(users[1].tenant, "DEFAULT");
    }

    #[test]
    fn rejects_empty_users() {
        assert!(parse_mgmt_users("").is_err());
    }
}
