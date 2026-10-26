//! The wasm transport: an extension is a `.wasm` module hawier loads and
//! calls.
//!
//! Chosen as the only transport because it is the one that makes the
//! capability rules real rather than aspirational. A wasm module has no
//! ambient authority at all: no filesystem, no sockets, no processes —
//! nothing but what the host hands it as an import. A native program, by
//! contrast, has everything its user has, and "the extension may not do IO"
//! is then a convention rather than a fact.
//!
//! What we hand it is deliberately thin: a clock, the `HAWIER_EXT_*`
//! environment variables, and — only if it asked for it in its manifest and
//! only to the hosts it named — the ability to have the host make an HTTP
//! request on its behalf. Not the database URL, not the internal token: an
//! extension cannot read the node's secrets even by accident.
//!
//! Note what `http_out` is and is not. It is a capability: the module never
//! holds a socket, it describes a request and the host performs it, which is
//! where the allowlist, the timeout and the size cap are enforced. It is not
//! a way to move payload — artifact bytes are still spooled and moved by the
//! host, and the cap makes sure of it.
//!
//! One instance cannot serve two calls at once — it has a single linear
//! memory and one buffer pair — so the host keeps a small pool of them and
//! hands one out per call. Serialising every call on a single instance
//! would put a global lock on the artifact path, where a fleet downloads
//! concurrently. Calls are bounded by an epoch deadline, and an instance
//! that traps or overruns is dropped rather than returned to the pool.

use std::time::Duration;

use hawier_api::ext::{
    ExtManifest, ExtRequest, ExtResponse, HttpOutcome, HttpRequest, HttpResponse,
};
use serde_json::Value;
use tokio::sync::{Mutex, Semaphore};
use wasmtime::{Engine, Linker, Memory, Module, Store, TypedFunc};
use wasmtime_wasi::p1::{self, WasiP1Ctx};
use wasmtime_wasi::WasiCtxBuilder;

use crate::error::{AppError, AppResult};

/// Epoch ticks are this long, so a deadline has this granularity.
const EPOCH_TICK: Duration = Duration::from_millis(10);
/// How long the host will wait on a request made for an extension.
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
/// Cap on a module's linear memory. An extension trades in decisions; one
/// that wants hundreds of megabytes is misbehaving.
const MEMORY_LIMIT: usize = 64 * 1024 * 1024;
/// How many calls may run at once. Instances are cheap, but not free.
const MAX_CONCURRENT_CALLS: usize = 32;
/// How many instances stay warm between bursts.
const IDLE_INSTANCES: usize = 8;

struct HostState {
    wasi: WasiP1Ctx,
    limits: wasmtime::StoreLimits,
    /// Hosts this module declared and may therefore reach. Empty means it
    /// asked for nothing, and every request is refused.
    allowed_hosts: Vec<String>,
    http: reqwest::Client,
}

/// Does `url` point at a host this module declared?
///
/// Exact host match, no suffix matching: `evil-example.com` must not pass
/// because someone allowed `example.com`.
fn host_allowed(url: &str, allowed: &[String]) -> bool {
    let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    else {
        return false;
    };
    let host = rest
        .split('/')
        .next()
        .unwrap_or_default()
        .split('@')
        .next_back()
        .unwrap_or_default();
    allowed.iter().any(|candidate| candidate == host)
}

/// A loaded module and the instance currently serving calls.
struct Running {
    store: Store<HostState>,
    memory: Memory,
    alloc: TypedFunc<u32, u32>,
    call: TypedFunc<(u32, u32), u64>,
    manifest: ExtManifest,
}

/// One engine for the whole process, with one epoch ticker behind it.
///
/// Engines are not cheap and each needs a thread driving its deadlines, so a
/// node running five extensions had five of both. Modules are still compiled
/// per extension; only the runtime they share.
fn shared_engine() -> AppResult<&'static Engine> {
    static ENGINE: std::sync::OnceLock<AppResult<Engine>> = std::sync::OnceLock::new();
    static TICKER: std::sync::Once = std::sync::Once::new();

    // The engine is built inside the OnceLock, and the ticker is started on
    // whichever engine was stored. Building it outside and then calling
    // `get_or_init` would, if two threads ever raced, keep the winner's
    // engine while the loser's thread ticked an orphan — deadlines would
    // simply never fire, silently. Nothing races today, but the manifest
    // handshake now runs at startup, so a module that hangs in
    // `hawier_manifest` would hang the boot forever without a live ticker.
    let engine = ENGINE
        .get_or_init(|| {
            let mut config = wasmtime::Config::new();
            config.epoch_interruption(true);
            Engine::new(&config).map_err(|e| {
                AppError::Internal(anyhow::Error::from(e).context("building the wasm engine"))
            })
        })
        .as_ref()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;

    let mut spawned = Ok(());
    TICKER.call_once(|| {
        // Without this an extension could spin forever and take the node
        // with it.
        let ticker = engine.clone();
        spawned = std::thread::Builder::new()
            .name("hawier-ext-epoch".to_string())
            .spawn(move || loop {
                std::thread::sleep(EPOCH_TICK);
                ticker.increment_epoch();
            })
            .map(|_| ())
            .map_err(|e| AppError::Internal(anyhow::Error::from(e)));
    });
    spawned?;
    Ok(engine)
}

pub struct Extension {
    /// The point this module was configured for, for error messages.
    point: String,
    path: String,
    deadline: Duration,
    engine: &'static Engine,
    module: Module,
    /// Passed to the guest: only extension parameters, never the node's
    /// secrets. Handed in rather than read from the environment here, so the
    /// snapshot is one fact taken once, and nothing can re-read it per call.
    env: Vec<(String, String)>,
    /// What the module announced at load. Read once, at load, so it is
    /// available before anything has been asked of the module — an inventory
    /// on a node with no rollouts and no retention rules would otherwise
    /// report "unknown" for those slots forever.
    manifest: ExtManifest,
    /// Instances ready for reuse. Capped: a burst may create more than this
    /// many, but only this many are kept warm afterwards.
    idle: Mutex<Vec<Running>>,
    /// Bounds how many calls run at once, and therefore how many instances
    /// can exist. Each call is microseconds, so waiting here is cheap.
    permits: Semaphore,
}

impl Extension {
    pub fn new(
        point: &str,
        path: &str,
        deadline: Duration,
        env: Vec<(String, String)>,
    ) -> AppResult<Self> {
        let engine = shared_engine()?;

        let module = Module::from_file(engine, path).map_err(|e| {
            AppError::Internal(
                anyhow::Error::from(e).context(format!("loading the wasm extension {path}")),
            )
        })?;

        let extension = Self {
            point: point.to_string(),
            path: path.to_string(),
            deadline,
            engine,
            module,
            env,
            // Replaced immediately below; `build` needs a constructed
            // Extension to read `env` and `deadline` from.
            manifest: ExtManifest {
                name: String::new(),
                version: String::new(),
                points: Vec::new(),
                sites: Vec::new(),
                capabilities: Vec::new(),
                deadline_ms: 0,
            },
            idle: Mutex::new(Vec::new()),
            permits: Semaphore::new(MAX_CONCURRENT_CALLS),
        };

        // Instantiate once, now. Two things follow from doing it here rather
        // than on the first call: the manifest is available to an inventory
        // straight away, and a module that cannot be instantiated — or whose
        // manifest does not implement the point it was configured for — takes
        // the node down at startup instead of at whatever hour the first
        // artifact download arrives. The instance is kept, not thrown away,
        // so the first real call finds it warm.
        let running = extension.build()?;
        let manifest = running.manifest.clone();
        // The pool is built with the instance already in it rather than
        // locked and pushed: this runs inside the async startup, and taking
        // a tokio mutex with `blocking_lock` there panics.
        Ok(Self {
            manifest,
            idle: Mutex::new(vec![running]),
            ..extension
        })
    }

    /// Call the extension, instantiating it if the pool is empty.
    ///
    /// Any failure — instantiation, a trap, the deadline, a malformed answer
    /// — discards the instance so the next call starts clean, and surfaces as
    /// `Unavailable`: the caller then does whatever the point says happens
    /// when there is no extension. It never substitutes an answer.
    pub async fn call(&self, request: ExtRequest) -> AppResult<Value> {
        let _permit = self
            .permits
            .acquire()
            .await
            .map_err(|_| self.unavailable("the extension was shut down"))?;

        let mut running = match self.idle.lock().await.pop() {
            Some(running) => running,
            None => self.instantiate()?,
        };
        // The deadline is absolute in epoch ticks, so it is renewed per call.
        running.store.set_epoch_deadline(self.deadline_ticks());

        match exchange(&mut running, &request) {
            Ok(ExtResponse::Ok { payload }) => {
                self.recycle(running).await;
                Ok(payload)
            }
            Ok(ExtResponse::Err { message }) => {
                // The module answered and refused; it is alive and in sync,
                // so keep it and report what it said.
                self.recycle(running).await;
                Err(AppError::Internal(anyhow::anyhow!(
                    "extension {} refused {}: {message}",
                    self.path,
                    request.point
                )))
            }
            Err(error) => {
                // Dropped, not reused: after a trap or a deadline the
                // instance's memory is not in a state we can trust.
                drop(running);
                Err(self.unavailable(&format!("{error:#}")))
            }
        }
    }

    async fn recycle(&self, running: Running) {
        let mut idle = self.idle.lock().await;
        if idle.len() < IDLE_INSTANCES {
            idle.push(running);
        }
    }

    /// What the module announced when it was loaded.
    pub fn manifest(&self) -> &ExtManifest {
        &self.manifest
    }

    /// The point this module was configured for.
    pub fn point(&self) -> &str {
        &self.point
    }

    /// The module file this node loaded.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The deadline the host enforces, which is the node's, not the
    /// module's: a module declares one in its manifest and the host has
    /// never used it. The inventory shows both so the difference is visible
    /// rather than surprising.
    pub fn deadline(&self) -> Duration {
        self.deadline
    }

    fn unavailable(&self, why: &str) -> AppError {
        tracing::warn!(
            extension = %self.path,
            point = %self.point,
            reason = why,
            "extension unavailable"
        );
        AppError::Unavailable(format!("extension for {} is unavailable", self.point))
    }

    fn instantiate(&self) -> AppResult<Running> {
        self.build()
            .map_err(|e| self.unavailable(&format!("{e:#}")))
    }

    fn build(&self) -> anyhow::Result<Running> {
        // Everything the guest gets. No preopened directory, no socket, no
        // inherited stdio beyond stderr for its own diagnostics.
        let wasi = WasiCtxBuilder::new()
            .inherit_stderr()
            .envs(&self.env)
            .build_p1();
        let limits = wasmtime::StoreLimitsBuilder::new()
            .memory_size(MEMORY_LIMIT)
            .build();
        let mut store = Store::new(
            self.engine,
            HostState {
                wasi,
                limits,
                // Filled in from the manifest once the handshake has run: a
                // module cannot be trusted with a capability it has not yet
                // told us it wants.
                allowed_hosts: Vec::new(),
                http: reqwest::Client::builder()
                    .timeout(HTTP_TIMEOUT)
                    .build()
                    .unwrap_or_default(),
            },
        );
        store.limiter(|state| &mut state.limits);
        store.set_epoch_deadline(self.deadline_ticks());

        let mut linker: Linker<HostState> = Linker::new(self.engine);
        p1::add_to_linker_sync(&mut linker, |state: &mut HostState| &mut state.wasi)?;
        add_http_out(&mut linker)?;
        let instance = linker.instantiate(&mut store, &self.module)?;

        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| anyhow::anyhow!("the module exports no memory"))?;
        let alloc: TypedFunc<u32, u32> = instance.get_typed_func(&mut store, "hawier_alloc")?;
        let call: TypedFunc<(u32, u32), u64> =
            instance.get_typed_func(&mut store, "hawier_call")?;
        let manifest_fn: TypedFunc<(), u64> =
            instance.get_typed_func(&mut store, "hawier_manifest")?;

        // Handshake: read the manifest before anything is asked of the
        // module, so one that does not implement this point fails here
        // rather than on the first real call.
        let packed = manifest_fn.call(&mut store, ())?;
        let manifest: ExtManifest = read_json(&memory, &mut store, packed)?;
        anyhow::ensure!(
            manifest.points.iter().any(|p| p == &self.point),
            "{} implements {:?}, not {}",
            manifest.name,
            manifest.points,
            self.point
        );
        // Only now, and only what it asked for.
        let allowed_hosts: Vec<String> = manifest
            .capabilities
            .iter()
            .flat_map(|capability| match capability {
                hawier_api::ext::Capability::HttpOut { hosts } => hosts.clone(),
            })
            .collect();
        if !allowed_hosts.is_empty() {
            tracing::info!(
                extension = %manifest.name,
                hosts = ?allowed_hosts,
                "extension may reach these hosts"
            );
        }
        store.data_mut().allowed_hosts = allowed_hosts;

        tracing::info!(
            extension = %manifest.name,
            version = %manifest.version,
            point = %self.point,
            "extension ready"
        );

        Ok(Running {
            store,
            memory,
            alloc,
            call,
            manifest,
        })
    }

    fn deadline_ticks(&self) -> u64 {
        (self.deadline.as_millis() as u64 / EPOCH_TICK.as_millis() as u64).max(1)
    }
}

fn exchange(running: &mut Running, request: &ExtRequest) -> anyhow::Result<ExtResponse> {
    let body = serde_json::to_vec(request)?;
    let len = u32::try_from(body.len())?;

    let ptr = running.alloc.call(&mut running.store, len)?;
    running
        .memory
        .write(&mut running.store, ptr as usize, &body)?;

    let packed = running.call.call(&mut running.store, (ptr, len))?;
    read_json(&running.memory, &mut running.store, packed)
}

/// Read a `(ptr << 32) | len` answer out of the guest's memory.
fn read_json<T: serde::de::DeserializeOwned>(
    memory: &Memory,
    store: &mut Store<HostState>,
    packed: u64,
) -> anyhow::Result<T> {
    let ptr = (packed >> 32) as usize;
    let len = (packed & 0xffff_ffff) as usize;
    anyhow::ensure!(len > 0, "the extension answered with an empty frame");
    anyhow::ensure!(
        len <= hawier_api::ext::MAX_ANSWER,
        "the extension answered with {len} bytes, over the cap"
    );

    let data = memory.data(&*store);
    let end = ptr
        .checked_add(len)
        .ok_or_else(|| anyhow::anyhow!("the extension answered with an out-of-range pointer"))?;
    anyhow::ensure!(
        end <= data.len(),
        "the extension answered outside its own memory"
    );
    Ok(serde_json::from_slice(&data[ptr..end])?)
}

/// The one import a module gets beyond WASI: "make this request for me".
///
/// Everything that decides whether it happens is on this side — the declared
/// capability, the host allowlist, the timeout, the size cap. The guest only
/// describes what it wants, so a module cannot reach anything its manifest
/// did not name, however it was written.
fn add_http_out(linker: &mut Linker<HostState>) -> anyhow::Result<()> {
    linker.func_wrap(
        "hawier",
        "hawier_http",
        |mut caller: wasmtime::Caller<'_, HostState>,
         req_ptr: u32,
         req_len: u32,
         out_ptr: u32,
         out_cap: u32|
         -> i64 {
            let memory = match caller.get_export("memory").and_then(|e| e.into_memory()) {
                Some(memory) => memory,
                None => return -1,
            };
            let outcome = perform_for_guest(&mut caller, memory, req_ptr, req_len);
            let encoded = serde_json::to_vec(&outcome).unwrap_or_default();
            if encoded.len() > out_cap as usize {
                // Say so in the buffer we were given rather than truncating
                // into something that will not parse.
                let short = serde_json::to_vec(&HttpOutcome::Refused {
                    reason: "the answer does not fit the buffer".to_string(),
                })
                .unwrap_or_default();
                if short.len() > out_cap as usize
                    || memory.write(&mut caller, out_ptr as usize, &short).is_err()
                {
                    return -1;
                }
                return short.len() as i64;
            }
            if memory
                .write(&mut caller, out_ptr as usize, &encoded)
                .is_err()
            {
                return -1;
            }
            encoded.len() as i64
        },
    )?;
    Ok(())
}

fn perform_for_guest(
    caller: &mut wasmtime::Caller<'_, HostState>,
    memory: Memory,
    req_ptr: u32,
    req_len: u32,
) -> HttpOutcome {
    if req_len as usize > hawier_api::ext::MAX_HTTP_BODY {
        return HttpOutcome::Refused {
            reason: "request is too large".to_string(),
        };
    }
    let mut raw = vec![0u8; req_len as usize];
    if memory.read(&*caller, req_ptr as usize, &mut raw).is_err() {
        return HttpOutcome::Refused {
            reason: "the request is outside the module's memory".to_string(),
        };
    }
    let request: HttpRequest = match serde_json::from_slice(&raw) {
        Ok(request) => request,
        Err(error) => {
            return HttpOutcome::Refused {
                reason: format!("unusable request: {error}"),
            }
        }
    };

    let allowed = caller.data().allowed_hosts.clone();
    if allowed.is_empty() {
        return HttpOutcome::Refused {
            reason: "this extension did not declare the http_out capability".to_string(),
        };
    }
    if !host_allowed(&request.url, &allowed) {
        return HttpOutcome::Refused {
            reason: format!("{} is not among the declared hosts", request.url),
        };
    }
    if request
        .body
        .as_ref()
        .is_some_and(|body| body.len() > hawier_api::ext::MAX_HTTP_BODY)
    {
        return HttpOutcome::Refused {
            reason: "request body is too large".to_string(),
        };
    }

    let client = caller.data().http.clone();
    // A guest call is synchronous, so the calling thread waits here. That is
    // inherent to the ABI and is a real cost of `http_out` — one more reason
    // the descriptor model stays the default for anything that does not
    // genuinely need to ask a question.
    //
    // The work goes to a runtime of our own rather than the caller's: a host
    // function must not care whether it was entered from a single-threaded
    // runtime or a worker pool, and a runtime cannot be driven from inside
    // another one.
    let (done, wait) = std::sync::mpsc::channel();
    http_runtime().spawn(async move {
        let outcome: Result<HttpResponse, String> = async move {
            let method = reqwest::Method::from_bytes(request.method.as_bytes())
                .map_err(|_| format!("unsupported method '{}'", request.method))?;
            let mut outgoing = client.request(method, &request.url);
            for (name, value) in &request.headers {
                outgoing = outgoing.header(name, value);
            }
            if let Some(body) = request.body {
                outgoing = outgoing.body(body);
            }
            let response = outgoing.send().await.map_err(|e| e.to_string())?;
            let status = response.status().as_u16();
            let headers = response
                .headers()
                .iter()
                .filter_map(|(name, value)| {
                    value
                        .to_str()
                        .ok()
                        .map(|value| (name.to_string(), value.to_string()))
                })
                .collect();
            let body = response.text().await.map_err(|e| e.to_string())?;
            if body.len() > hawier_api::ext::MAX_HTTP_BODY {
                return Err("response body is too large".to_string());
            }
            Ok(HttpResponse {
                status,
                headers,
                body,
            })
        }
        .await;
        // The receiver is gone only if this call was abandoned; nothing to do.
        let _ = done.send(outcome);
    });

    // Slightly past the client's own timeout, so a hung request surfaces as
    // a failure rather than a thread parked forever.
    match wait.recv_timeout(HTTP_TIMEOUT + Duration::from_secs(5)) {
        Ok(Ok(response)) => HttpOutcome::Ok(response),
        Ok(Err(reason)) => HttpOutcome::Failed { reason },
        Err(_) => HttpOutcome::Failed {
            reason: "the request did not finish in time".to_string(),
        },
    }
}

/// A runtime dedicated to requests made on behalf of extensions.
///
/// One worker: these are rare, and giving them their own runtime keeps them
/// off the threads serving devices.
fn http_runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .thread_name("hawier-ext-http")
            .build()
            .expect("extension http runtime")
    })
}

#[cfg(test)]
mod tests {
    use super::host_allowed;

    #[test]
    fn only_the_declared_host_passes() {
        let allowed = vec!["issuer.example".to_string()];
        assert!(host_allowed("https://issuer.example/keys", &allowed));
        assert!(host_allowed(
            "http://issuer.example:8080/keys",
            &["issuer.example:8080".to_string()]
        ));
    }

    #[test]
    fn a_suffix_is_not_a_match() {
        // Allowing `example.com` must not hand over `evil-example.com`, which
        // is exactly the mistake a `ends_with` check makes.
        let allowed = vec!["example.com".to_string()];
        assert!(!host_allowed("https://evil-example.com/keys", &allowed));
        assert!(!host_allowed("https://example.com.evil.net/keys", &allowed));
    }

    #[test]
    fn credentials_in_the_url_do_not_disguise_the_host() {
        let allowed = vec!["issuer.example".to_string()];
        assert!(!host_allowed(
            "https://issuer.example@evil.net/keys",
            &allowed
        ));
    }

    #[test]
    fn a_scheme_we_do_not_speak_is_refused() {
        let allowed = vec!["issuer.example".to_string()];
        assert!(!host_allowed("file:///etc/passwd", &allowed));
        assert!(!host_allowed("issuer.example/keys", &allowed));
    }
}
