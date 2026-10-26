//! Wire types for hawier's extension mechanism (see EXTENSIONS.md).
//!
//! The one ABI invariant lives here in type form: what crosses the boundary
//! to an extension is a **decision or a descriptor**, never bytes and never
//! a handle. So everything in this module is a plain serde value — nothing
//! borrows a connection, a stream or a file.
//!
//! These types are shared because a guest (subprocess today, wasm later)
//! speaks them over a pipe, and the host speaks the same shapes in-process.
//! A `builtin` extension is not a different contract, only a different
//! transport.

use serde::{Deserialize, Serialize};

/// What happened, as told to the outside world.
///
/// Deliberately a flat, self-describing envelope rather than a Rust enum
/// with typed payloads per kind: subscribers filter on `kind` as a string,
/// a guest deserialises it in whatever language it is written in, and
/// adding a kind must not be a breaking change for anyone.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainEvent {
    /// Decimal snowflake, same shape as every other hawier id.
    pub id: String,
    pub kind: String,
    pub tenant: String,
    /// Epoch milliseconds — every timestamp on hawier's wire is millis.
    pub occurred_at: i64,
    /// Kind-specific fields: ids and names of what changed, never payloads.
    pub subject: serde_json::Value,
}

/// The event kinds hawier emits. String constants rather than an enum so a
/// subscriber's `event_mask` and the emitting call site cannot drift apart
/// without the compiler noticing at the call site.
pub mod kinds {
    pub const ROLLOUT_CREATED: &str = "rollout.created";
    pub const ROLLOUT_STARTED: &str = "rollout.started";
    pub const ROLLOUT_PAUSED: &str = "rollout.paused";
    pub const ROLLOUT_FINISHED: &str = "rollout.finished";
    pub const ASSIGNMENT_CREATED: &str = "assignment.created";
    pub const ARTIFACT_UPLOADED: &str = "artifact.uploaded";
    pub const ARTIFACT_DELETED: &str = "artifact.deleted";

    /// Every kind, for validating an `event_mask` against reality.
    pub const ALL: &[&str] = &[
        ROLLOUT_CREATED,
        ROLLOUT_STARTED,
        ROLLOUT_PAUSED,
        ROLLOUT_FINISHED,
        ASSIGNMENT_CREATED,
        ARTIFACT_UPLOADED,
        ARTIFACT_DELETED,
    ];
}

/// How an `EventSink` wants an event delivered.
///
/// The sink returns a description of the request; the **host** performs it.
/// That keeps a sink a pure function — testable, and safe to run as a guest
/// without granting it network access at all. A sink that genuinely needs
/// to talk to the world itself must ask for the `http_out` capability, and
/// that is the exception, not the shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Delivery {
    pub url: String,
    #[serde(default = "post")]
    pub method: String,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    pub body: String,
}

fn post() -> String {
    "POST".to_string()
}

/// Where an extension point runs, and therefore what it may cost and what
/// happens when it is unavailable. See EXTENSIONS.md §3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionSite {
    /// Background loop on the control node.
    Control,
    /// On a mgmt/UI request, under a tight deadline and a cache.
    Request,
    /// On the device path, including sub nodes: must be cacheable, must
    /// have a declared fallback, and must work with control down.
    Ddi,
}

/// What an extension declares about itself before it is allowed to run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtManifest {
    pub name: String,
    pub version: String,
    /// Extension points this implements, e.g. `event_sink`.
    pub points: Vec<String>,
    #[serde(default)]
    pub sites: Vec<ExecutionSite>,
    #[serde(default)]
    pub capabilities: Vec<Capability>,
    #[serde(default = "default_deadline_ms")]
    pub deadline_ms: u64,
}

fn default_deadline_ms() -> u64 {
    2_000
}

/// Ambient authority an extension has to ask for; nothing is granted by
/// default. Most points — s3 presigning, event sinks that describe their
/// delivery — need none of it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Capability {
    /// Outbound HTTP, restricted to these hosts.
    HttpOut { hosts: Vec<String> },
}

/// One call to a guest: which point, and the argument.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtRequest {
    pub point: String,
    pub payload: serde_json::Value,
}

/// A guest's answer. `Err` is a refusal or a failure by the extension
/// itself; a transport failure never reaches this type — the host turns
/// that into the point's declared fallback.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtResponse {
    Ok { payload: serde_json::Value },
    Err { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivery_defaults_to_post() {
        let delivery: Delivery =
            serde_json::from_str(r#"{"url":"https://example/hook","body":"{}"}"#).unwrap();
        assert_eq!(delivery.method, "POST");
        assert!(delivery.headers.is_empty());
    }

    #[test]
    fn every_point_is_listed_in_all() {
        // ALL is what an inventory walks, so a point missing from it is a
        // point the console cannot report as configured or absent.
        for point in [
            points::STORAGE_LOCATE,
            points::EVENT_SINK_PLAN,
            points::RETENTION_SELECT,
            points::ROLLOUT_PLAN,
            points::ROLLOUT_EVALUATE,
            points::CONSOLE_SURFACES,
            points::AUTHZ_DECIDE,
        ] {
            assert!(points::ALL.contains(&point), "{point} is missing from ALL");
        }
        assert_eq!(
            points::ALL.len(),
            points::ALL
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            "duplicate point in ALL"
        );
    }

    #[test]
    fn every_kind_is_listed_in_all() {
        // ALL is what validates a subscription mask, so a kind missing from
        // it would be un-subscribable while still being emitted.
        assert!(kinds::ALL.contains(&kinds::ROLLOUT_CREATED));
        assert_eq!(
            kinds::ALL.len(),
            kinds::ALL
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            "duplicate kind in ALL"
        );
    }

    #[test]
    fn execution_site_is_lowercase_on_the_wire() {
        assert_eq!(
            serde_json::to_string(&ExecutionSite::Ddi).unwrap(),
            "\"ddi\""
        );
    }
}

/// Cap on an answer read out of a guest's memory. An extension trades in
/// decisions, so nothing legitimate is large, and a bogus length must not
/// make the host allocate on the guest's say-so.
pub const MAX_ANSWER: usize = 8 * 1024 * 1024;

/// Extension point names, shared so host and guest cannot disagree on the
/// spelling of the thing they are talking about.
pub mod points {
    /// Where an artifact lives: `StorageLocate` in, `StorageLocation` out.
    pub const STORAGE_LOCATE: &str = "storage.locate";
    /// What to send for an event: `SinkPlan` in, `Vec<PlannedDelivery>` out.
    pub const EVENT_SINK_PLAN: &str = "event_sink.plan";
    /// Which of these may go: `RetentionRequest` in, `RetentionPlan` out.
    pub const RETENTION_SELECT: &str = "retention.select";
    /// How to divide a rollout: `RolloutPlanInput` in, `GroupPlan` out.
    pub const ROLLOUT_PLAN: &str = "rollout.plan";
    /// Whether a running rollout may move: `RolloutStats` in,
    /// `RolloutVerdict` out.
    pub const ROLLOUT_EVALUATE: &str = "rollout.evaluate";
    /// What an extension puts in the console tree: no input, a
    /// `ConsoleDeclaration` out.
    pub const CONSOLE_SURFACES: &str = "console.surfaces";
    /// May this identity do this: `AuthzRequest` in, `AuthzVerdict` out.
    pub const AUTHZ_DECIDE: &str = "authz.decide";

    /// Every point, so an inventory cannot silently omit one. A point added
    /// here and forgotten below would be a capability the node has and the
    /// operator cannot see.
    pub const ALL: &[&str] = &[
        STORAGE_LOCATE,
        EVENT_SINK_PLAN,
        RETENTION_SELECT,
        ROLLOUT_PLAN,
        ROLLOUT_EVALUATE,
        CONSOLE_SURFACES,
        AUTHZ_DECIDE,
    ];
}

/// Which operation on an artifact a location is wanted for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StorageOp {
    Get,
    Put,
    Head,
    Delete,
}

/// Ask an extension where an artifact is.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageLocate {
    pub tenant: String,
    pub sha1: String,
    pub op: StorageOp,
}

/// Where it is — a descriptor, never the bytes themselves.
///
/// `Url` covers object stores (the host redirects a device to it, or performs
/// the transfer itself). `Path` covers a filesystem: the host opens it, so
/// the extension still never touches a byte. The host refuses a path outside
/// the root it was configured with — a descriptor is data, not authority.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StorageLocation {
    Url {
        url: String,
        #[serde(default = "get")]
        method: String,
        #[serde(default)]
        headers: Vec<(String, String)>,
        /// Epoch millis after which the URL stops working, if it expires.
        #[serde(default)]
        expires_at: Option<i64>,
        /// May the host hand this straight to a device as a redirect?
        #[serde(default)]
        redirect_ok: bool,
    },
    Path {
        path: String,
    },
    /// The artifact is not there.
    Absent,
}

fn get() -> String {
    "GET".to_string()
}

/// Ask a sink what to send for one event, given the subscriptions the host
/// holds. The host owns the database; the extension decides.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SinkPlan {
    pub event: DomainEvent,
    pub subscriptions: Vec<Subscription>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    /// Decimal snowflake, as a string like every other id on the wire.
    pub id: String,
    pub url: String,
    #[serde(default)]
    pub secret: String,
    #[serde(default)]
    pub event_mask: String,
}

/// One delivery the sink wants performed, and whose subscription it is.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedDelivery {
    pub subscription_id: String,
    pub delivery: Delivery,
}

/// HMAC-SHA256 (RFC 2104).
///
/// Lives here because it is part of the wire contract, not of any one
/// program: an S3 extension signs requests with it, a webhook extension
/// signs bodies with it, and a subscriber recomputes it to check that a
/// delivery really came from hawier. Pure function, no IO — fifteen lines
/// over `sha2`, so no crate is worth adding for it.
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    const BLOCK: usize = 64;

    let mut padded = [0u8; BLOCK];
    if key.len() > BLOCK {
        padded[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        padded[..key.len()].copy_from_slice(key);
    }

    let mut inner_pad = [0x36u8; BLOCK];
    let mut outer_pad = [0x5cu8; BLOCK];
    for i in 0..BLOCK {
        inner_pad[i] ^= padded[i];
        outer_pad[i] ^= padded[i];
    }

    let inner = Sha256::new()
        .chain_update(inner_pad)
        .chain_update(message)
        .finalize();
    Sha256::new()
        .chain_update(outer_pad)
        .chain_update(inner)
        .finalize()
        .into()
}

#[cfg(test)]
mod hmac_tests {
    use super::hmac_sha256;

    /// RFC 4231 test case 1.
    #[test]
    fn matches_rfc4231_case_one() {
        assert_eq!(
            hex::encode(hmac_sha256(&[0x0b; 20], b"Hi There")),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    /// RFC 4231 test case 2.
    #[test]
    fn matches_rfc4231_case_two() {
        assert_eq!(
            hex::encode(hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    /// RFC 4231 test case 3.
    #[test]
    fn matches_rfc4231_case_three() {
        assert_eq!(
            hex::encode(hmac_sha256(&[0xaa; 20], &[0xdd; 50])),
            "773ea91e36800e46854db8ebd09181a72959098b3ef8c122d9635514ced565fe"
        );
    }

    /// RFC 4231 test case 6 — a key longer than the block, so the
    /// key-hashing branch runs.
    #[test]
    fn hashes_oversized_keys() {
        assert_eq!(
            hex::encode(hmac_sha256(
                &[0xaa; 131],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            )),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }
}

/// Generate the wasm exports an extension module must provide.
///
/// The whole guest side of the transport, so an extension is just two
/// functions: one that returns its manifest, one that answers a request.
/// Both speak the same JSON as everything else here — the contract does not
/// change with the transport, only who executes it.
///
/// The ABI is four exports:
///
/// - `memory`
/// - `hawier_alloc(len) -> ptr` — host writes the request there
/// - `hawier_manifest() -> (ptr << 32) | len`
/// - `hawier_call(ptr, len) -> (ptr << 32) | len`
///
/// Both buffers are owned by the guest and reused between calls, so nothing
/// leaks and no `free` export is needed. The host serialises calls per
/// instance, so a buffer stays valid until the next call.
///
/// Sketch — `text` rather than `ignore` on purpose: `make it` runs
/// `cargo test -- --ignored`, which would otherwise pick this up and try to
/// compile it.
///
/// ```text
/// fn manifest() -> ExtManifest { .. }
/// fn call(request: ExtRequest) -> ExtResponse { .. }
/// hawier_api::wasm_extension!(manifest, call);
/// ```
#[macro_export]
macro_rules! wasm_extension {
    ($manifest:path, $call:path) => {
        #[doc(hidden)]
        mod __hawier_wasm_abi {
            use super::*;

            thread_local! {
                /// Where the host writes a request.
                static INPUT: ::std::cell::RefCell<::std::vec::Vec<u8>> =
                    ::std::cell::RefCell::new(::std::vec::Vec::new());
                /// Where the guest leaves an answer for the host to read.
                static OUTPUT: ::std::cell::RefCell<::std::vec::Vec<u8>> =
                    ::std::cell::RefCell::new(::std::vec::Vec::new());
            }

            /// Pack a pointer and a length into the single i64 wasm returns.
            fn hand_back(bytes: ::std::vec::Vec<u8>) -> u64 {
                OUTPUT.with(|buffer| {
                    let mut buffer = buffer.borrow_mut();
                    *buffer = bytes;
                    ((buffer.as_ptr() as u64) << 32) | (buffer.len() as u64)
                })
            }

            #[no_mangle]
            pub extern "C" fn hawier_alloc(len: u32) -> u32 {
                INPUT.with(|buffer| {
                    let mut buffer = buffer.borrow_mut();
                    buffer.clear();
                    buffer.resize(len as usize, 0);
                    buffer.as_ptr() as u32
                })
            }

            #[no_mangle]
            pub extern "C" fn hawier_manifest() -> u64 {
                let manifest = $manifest();
                match $crate::ext::__serialise(&manifest) {
                    Ok(bytes) => hand_back(bytes),
                    // Nothing can be reported through a manifest that will
                    // not serialise; an empty answer fails the handshake.
                    Err(_) => hand_back(::std::vec::Vec::new()),
                }
            }

            #[no_mangle]
            pub extern "C" fn hawier_call(ptr: u32, len: u32) -> u64 {
                // Safety: this is the buffer `hawier_alloc` just handed the
                // host, still owned by INPUT and not reallocated since.
                let body = unsafe { ::std::slice::from_raw_parts(ptr as *const u8, len as usize) };
                let response = match $crate::ext::__parse::<$crate::ext::ExtRequest>(body) {
                    Ok(request) => $call(request),
                    Err(message) => $crate::ext::ExtResponse::Err { message },
                };
                match $crate::ext::__serialise(&response) {
                    Ok(bytes) => hand_back(bytes),
                    Err(message) => hand_back(
                        $crate::ext::__serialise(&$crate::ext::ExtResponse::Err { message })
                            .unwrap_or_default(),
                    ),
                }
            }
        }
    };
}

/// Serialise for the wasm ABI. Public only so [`wasm_extension`] can use it.
#[doc(hidden)]
pub fn __serialise<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    serde_json::to_vec(value).map_err(|e| format!("encoding: {e}"))
}

/// Parse for the wasm ABI. Public only so [`wasm_extension`] can use it.
#[doc(hidden)]
pub fn __parse<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, String> {
    serde_json::from_slice(bytes).map_err(|e| format!("decoding: {e}"))
}

// ---------------------------------------------------------------------------
// Retention — what may be deleted
// ---------------------------------------------------------------------------

/// One thing a retention rule is asking about.
///
/// Enough to judge by, and nothing else: no bytes, no handles. The flags are
/// the questions a policy actually asks and RSQL cannot answer, so the host
/// computes them once rather than making every extension re-derive them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionCandidate {
    /// Artifact id, decimal snowflake as a string like every other id.
    pub id: String,
    pub sha1: String,
    pub filename: String,
    pub size: i64,
    /// Epoch milliseconds.
    pub created_at: i64,
    pub software_module: String,
    pub module_name: String,
    pub module_version: String,
    /// The module belongs to at least one distribution set.
    pub in_distribution_set: bool,
    /// Some device was, at some point, told to install it.
    pub ever_assigned: bool,
}

/// A rule firing: what it is, and everything it may consider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionRequest {
    /// `artifact` today; the field exists so a rule over something else does
    /// not need a second point.
    pub scope: String,
    pub rule: String,
    /// Rule-specific knobs, handed through verbatim.
    #[serde(default)]
    pub params: serde_json::Value,
    /// Epoch milliseconds, injected rather than read from a clock so a
    /// decision is a pure function of its input.
    pub now: i64,
    pub candidates: Vec<RetentionCandidate>,
}

/// What the extension decided.
///
/// Only ids from the candidate list are honoured — the host checks, because a
/// descriptor is data, not authority, and this one deletes things.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionPlan {
    /// Ids to delete. Anything not named here is kept: silence means keep,
    /// which is the safe direction for a rule that deletes.
    #[serde(default)]
    pub delete: Vec<String>,
    /// Shown to the operator beside the rule.
    #[serde(default)]
    pub reason: String,
}

// ---------------------------------------------------------------------------
// Rollout strategy — how a rollout is divided, and when it may move on
// ---------------------------------------------------------------------------

/// A group as the operator declared it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupDefinition {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub target_percentage: Option<f64>,
    /// The per-group filter hawkBit accepts. Whether it means anything is up
    /// to the strategy — the classic one ignores it, which is the behaviour
    /// recorded in API_NOTES §7.
    #[serde(default)]
    pub target_filter: String,
    pub success_threshold: f64,
    #[serde(default)]
    pub error_threshold: Option<f64>,
}

/// What the host knows when a rollout is about to be divided.
///
/// Note what is *not* here: the targets. A rollout can cover tens of millions
/// of devices, so the extension is told how many there are and answers with
/// selectors the host runs as SQL. Decisions and descriptors, never data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RolloutPlanInput {
    pub rollout: String,
    pub name: String,
    pub strategy: String,
    #[serde(default)]
    pub params: serde_json::Value,
    /// Targets matching the rollout's own filter.
    pub total_targets: i64,
    pub groups: Vec<GroupDefinition>,
}

/// How to fill one group.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GroupSelector {
    /// A share of whatever earlier groups left — hawkBit's own model.
    Share { percentage: f64 },
    /// RSQL over targets, intersected with the rollout's own filter.
    Filter { rsql: String },
    /// Targets whose stable bucket falls in `[from, to)`.
    ///
    /// The bucket is derived from the controller id, so it is the same on
    /// every node, needs nothing from the device, and does not move when a
    /// device re-registers.
    Bucket { from: i64, to: i64, modulo: i64 },
    /// Everything not taken by an earlier group.
    Rest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedGroup {
    /// The declared group this fills, when there is one. A strategy may also
    /// invent groups (a dynamic rollout growing another wave).
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    pub selector: GroupSelector,
    pub success_threshold: f64,
    #[serde(default)]
    pub error_threshold: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupPlan {
    pub groups: Vec<PlannedGroup>,
}

/// Where a running rollout stands. Counts, not rows.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupStats {
    pub id: String,
    pub name: String,
    pub total: i64,
    pub finished: i64,
    pub errors: i64,
    pub success_threshold: f64,
    #[serde(default)]
    pub error_threshold: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RolloutStats {
    pub rollout: String,
    pub strategy: String,
    #[serde(default)]
    pub params: serde_json::Value,
    pub dynamic: bool,
    /// Only the groups that are currently running.
    pub groups: Vec<GroupStats>,
}

/// What the strategy makes of it.
///
/// Only a judgement on the groups it was shown: the state machine — activate
/// the next group, grow a dynamic rollout, finish — stays in the host, so a
/// strategy cannot leave a rollout in a state the engine does not know.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RolloutVerdict {
    /// Groups that have met their success condition.
    #[serde(default)]
    pub finished: Vec<String>,
    /// Groups that have met their error condition.
    #[serde(default)]
    pub errored: Vec<String>,
    /// Stop the rollout, and tell the operator why.
    #[serde(default)]
    pub pause: Option<String>,
}

// ---------------------------------------------------------------------------
// Console surfaces — what an extension puts in the tree
// ---------------------------------------------------------------------------

/// A view under a resource: a saved query with a name, as a tree node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurfaceView {
    pub id: String,
    pub label: String,
    /// RSQL. Must name fields the resource's server-side map knows, or the
    /// view answers 400 and shows a red callout instead of a list.
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub sort: Option<String>,
    #[serde(default)]
    pub columns: Option<Vec<String>>,
}

/// A resource node in the tree, with its own views.
///
/// `endpoint` is a mgmt path the node already serves: an extension declares
/// how something is *presented*, not where the data comes from. Serving data
/// would need an extension point that answers HTTP, which does not exist.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsoleSurface {
    pub id: String,
    pub label: String,
    pub endpoint: String,
    /// `inventory`, `deployment`, `repository` or `admin`; anything else
    /// lands in admin rather than vanishing.
    #[serde(default)]
    pub group: String,
    #[serde(default = "id_field")]
    pub id_field: String,
    pub columns: Vec<String>,
    #[serde(default)]
    pub default_sort: Option<String>,
    #[serde(default)]
    pub search_fields: Vec<String>,
    /// Icon name from the console's set. Every resource needs its own —
    /// in a collapsed sidebar it is the only thing telling them apart.
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub views: Vec<SurfaceView>,
}

fn id_field() -> String {
    "id".to_string()
}

/// Views added to a resource that already exists, rather than a new node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsoleViews {
    pub resource: String,
    pub views: Vec<SurfaceView>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConsoleDeclaration {
    #[serde(default)]
    pub surfaces: Vec<ConsoleSurface>,
    #[serde(default)]
    pub views: Vec<ConsoleViews>,
}

// ---------------------------------------------------------------------------
// Authorisation — may this identity do this
// ---------------------------------------------------------------------------

/// The permissions hawier checks. hawkBit-shaped names, because operators
/// and existing tooling already know them.
pub mod permissions {
    pub const READ_TARGET: &str = "READ_TARGET";
    pub const UPDATE_TARGET: &str = "UPDATE_TARGET";
    pub const CREATE_TARGET: &str = "CREATE_TARGET";
    pub const DELETE_TARGET: &str = "DELETE_TARGET";
    pub const READ_REPOSITORY: &str = "READ_REPOSITORY";
    pub const UPDATE_REPOSITORY: &str = "UPDATE_REPOSITORY";
    pub const CREATE_REPOSITORY: &str = "CREATE_REPOSITORY";
    pub const DELETE_REPOSITORY: &str = "DELETE_REPOSITORY";
    pub const READ_ROLLOUT: &str = "READ_ROLLOUT";
    pub const CREATE_ROLLOUT: &str = "CREATE_ROLLOUT";
    pub const UPDATE_ROLLOUT: &str = "UPDATE_ROLLOUT";
    pub const DELETE_ROLLOUT: &str = "DELETE_ROLLOUT";
    pub const HANDLE_ROLLOUT: &str = "HANDLE_ROLLOUT";
    /// Administration of hawier's own machinery: webhooks, retention rules,
    /// roles. Separate from the repository so a release engineer is not
    /// automatically an administrator.
    pub const READ_SYSTEM: &str = "READ_SYSTEM";
    pub const UPDATE_SYSTEM: &str = "UPDATE_SYSTEM";
    /// Everything. What a config user gets, and what a tenant with no roles
    /// defined gets, so enforcement is something you switch on.
    pub const ALL: &str = "ROLE_ALL";

    pub const KNOWN: &[&str] = &[
        READ_TARGET,
        UPDATE_TARGET,
        CREATE_TARGET,
        DELETE_TARGET,
        READ_REPOSITORY,
        UPDATE_REPOSITORY,
        CREATE_REPOSITORY,
        DELETE_REPOSITORY,
        READ_ROLLOUT,
        CREATE_ROLLOUT,
        UPDATE_ROLLOUT,
        DELETE_ROLLOUT,
        HANDLE_ROLLOUT,
        READ_SYSTEM,
        UPDATE_SYSTEM,
        ALL,
    ];
}

/// Who is asking, what they hold, and what they want to do.
///
/// The host looks the roles and grants up — it owns the database — and the
/// extension decides. Plain RBAC is a lookup and needs nothing else; a policy
/// that also cares about the hour of the day, or refuses production tenants
/// on a Friday, is why this is a decision and not a query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthzRequest {
    pub username: String,
    pub tenant: String,
    /// Roles granted to this user in this tenant.
    pub roles: Vec<String>,
    /// The permissions those roles carry, already unioned by the host.
    pub granted: Vec<String>,
    /// What is being attempted.
    pub permission: String,
    /// `true` when the tenant has defined no roles at all. Enforcement is
    /// opt-in per tenant: a tenant that never configured RBAC keeps working
    /// exactly as before rather than locking everyone out on upgrade.
    pub unconfigured: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthzVerdict {
    pub allow: bool,
    /// Shown to the operator on a refusal, so "403" is not the whole story.
    #[serde(default)]
    pub reason: String,
}

// ---------------------------------------------------------------------------
// The http_out capability
// ---------------------------------------------------------------------------

/// A request an extension asks the host to make on its behalf.
///
/// The module never holds a socket. It describes a request; the host checks
/// the declared capability and its host allowlist, performs it, and hands
/// back the answer. So "may this extension reach the network" stays a
/// property the host enforces rather than a promise the author makes — and
/// an extension that genuinely needs to ask something (an issuer for its
/// keys, a signing service for a URL) can.
///
/// What this is *not* for is payload. Artifact bytes still never cross the
/// boundary: they are spooled and moved by the host, which is why an S3
/// upload is a presigned PUT the host performs rather than eight megabytes
/// through linear memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpRequest {
    pub url: String,
    #[serde(default = "get_method")]
    pub method: String,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    /// Request body, if any. Small by construction — see above.
    #[serde(default)]
    pub body: Option<String>,
}

fn get_method() -> String {
    "GET".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpResponse {
    pub status: u16,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// What came back, or why nothing did. A refusal is distinguishable from a
/// failed request: an extension reaching for a host it never declared is a
/// bug in the extension, not a flaky network.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HttpOutcome {
    Ok(HttpResponse),
    /// The host would not make this request: capability not declared, host
    /// not in the allowlist, body or response too large.
    Refused {
        reason: String,
    },
    /// The request was made and did not complete.
    Failed {
        reason: String,
    },
}

/// Cap on a body in either direction. Generous for a token or a signed URL,
/// far too small to be a way of smuggling artifacts through.
pub const MAX_HTTP_BODY: usize = 1024 * 1024;

/// Ask the host to make a request.
///
/// The ABI is deliberately one-way: the guest hands over a request and a
/// buffer, and the host fills the buffer. It never calls back into the
/// module, so a host function cannot re-enter an instance that is already
/// running — which is the shape of bug that only shows up under load.
///
/// Off wasm this is a stub that refuses, so an extension's own unit tests
/// compile and run on the host without a network.
#[cfg(target_arch = "wasm32")]
pub fn http(request: &HttpRequest) -> HttpOutcome {
    #[link(wasm_import_module = "hawier")]
    unsafe extern "C" {
        fn hawier_http(req_ptr: u32, req_len: u32, out_ptr: u32, out_cap: u32) -> i64;
    }

    let encoded = match serde_json::to_vec(request) {
        Ok(bytes) => bytes,
        Err(error) => {
            return HttpOutcome::Refused {
                reason: format!("cannot encode the request: {error}"),
            }
        }
    };
    // One buffer, reused: a request per call is normal, an allocation per
    // call for a megabyte is not.
    const CAPACITY: usize = MAX_HTTP_BODY + 64 * 1024;
    let mut out = vec![0u8; CAPACITY];
    let written = unsafe {
        hawier_http(
            encoded.as_ptr() as u32,
            encoded.len() as u32,
            out.as_mut_ptr() as u32,
            CAPACITY as u32,
        )
    };
    if written < 0 {
        return HttpOutcome::Refused {
            reason: "the host refused the call".to_string(),
        };
    }
    match serde_json::from_slice(&out[..written as usize]) {
        Ok(outcome) => outcome,
        Err(error) => HttpOutcome::Failed {
            reason: format!("unusable answer from the host: {error}"),
        },
    }
}

/// Host-side stub: extensions are wasm, and their unit tests are not.
#[cfg(not(target_arch = "wasm32"))]
pub fn http(_request: &HttpRequest) -> HttpOutcome {
    HttpOutcome::Refused {
        reason: "http_out is only available inside a wasm extension".to_string(),
    }
}
