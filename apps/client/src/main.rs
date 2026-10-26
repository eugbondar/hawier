use std::collections::BTreeMap;
use std::time::Duration;

use clap::Parser;
use hawier_client::{DdiAuth, DdiClient, Device, DeviceOptions};

/// hawier-device: DDI device simulator (rewrite of the hawkbit-sdk device).
#[derive(Parser, Debug, Clone)]
#[command(name = "hawier-device", version)]
struct Args {
    /// Base URL of the hawier/hawkBit DDI endpoint (the central node or an
    /// LB in front of the nodes). Everything past the initial poll follows
    /// server-provided `_links` — see API_NOTES.md.
    #[arg(
        long,
        default_value = "http://localhost:8080",
        env = "HAWIER_DEVICE_URL"
    )]
    url: String,
    #[arg(long, default_value = "DEFAULT")]
    tenant: String,
    #[arg(long, default_value = "hawier-device-1")]
    controller_id: String,
    /// Target security token (Authorization: TargetToken <hex>).
    #[arg(long)]
    token: Option<String>,
    /// Tenant gateway token (Authorization: GatewayToken <key>).
    #[arg(long)]
    gateway_token: Option<String>,
    /// Override the server-provided polling sleep, e.g. "5s" or "500ms".
    #[arg(long)]
    poll_override: Option<humantime_duration::Dur>,
    /// Fleet mode: run N devices with ids `{controller-id}-{n}`.
    #[arg(long, default_value_t = 1)]
    count: usize,
    /// Store downloads here instead of verify-and-discard.
    #[arg(long)]
    download_dir: Option<std::path::PathBuf>,
    /// Run one poll cycle and exit (integration tests use this).
    #[arg(long, default_value_t = false)]
    once: bool,
    /// Fault injection: close updates with failure.
    #[arg(long, default_value_t = false)]
    fail_download: bool,
    /// Deny confirmation requests instead of confirming them.
    #[arg(long, default_value_t = false)]
    deny_confirmation: bool,
    /// Polite mode: defer soft/attempt updates (download and wait until
    /// the server escalates to forced) instead of applying immediately.
    #[arg(long, default_value_t = false)]
    defer_soft: bool,
    /// Controller attributes to report, k=v comma-separated.
    #[arg(long, value_delimiter = ',')]
    attributes: Vec<String>,
    /// Benchmark mode: only poll the controller base at the given
    /// --poll-override interval and report POLLSTAT lines once a second.
    #[arg(long, default_value_t = false)]
    poll_only: bool,
}

/// Tiny "5s"/"500ms"/"2m" parser — not worth a crate dependency.
mod humantime_duration {
    use std::time::Duration;

    #[derive(Debug, Clone)]
    pub struct Dur(pub Duration);

    impl std::str::FromStr for Dur {
        type Err = String;

        fn from_str(s: &str) -> Result<Self, Self::Err> {
            let s = s.trim();
            let (value, unit) = s
                .find(|c: char| !c.is_ascii_digit())
                .map(|i| s.split_at(i))
                .ok_or_else(|| format!("missing unit in duration '{s}'"))?;
            let value: u64 = value.parse().map_err(|e| format!("{e}"))?;
            let duration = match unit {
                "ms" => Duration::from_millis(value),
                "s" => Duration::from_secs(value),
                "m" => Duration::from_secs(value * 60),
                "h" => Duration::from_secs(value * 3600),
                other => return Err(format!("unknown duration unit '{other}'")),
            };
            Ok(Dur(duration))
        }
    }
}

fn parse_attributes(raw: &[String]) -> BTreeMap<String, String> {
    raw.iter()
        .filter_map(|entry| entry.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "hawier_client=info".into()),
        )
        .init();

    let args = Args::parse();
    let poll_override: Option<Duration> = args.poll_override.clone().map(|d| d.0);

    if args.poll_only {
        return poll_only_fleet(&args, poll_override.unwrap_or(Duration::from_secs(1))).await;
    }

    let mut handles = Vec::new();
    for n in 0..args.count {
        let controller_id = if args.count == 1 {
            args.controller_id.clone()
        } else {
            format!("{}-{}", args.controller_id, n)
        };
        let auth = match (&args.token, &args.gateway_token) {
            (Some(t), _) => DdiAuth::TargetToken(t.clone()),
            (None, Some(g)) => DdiAuth::GatewayToken(g.clone()),
            (None, None) => DdiAuth::None,
        };
        let client = DdiClient::new(args.url.clone(), args.tenant.clone(), controller_id, auth);
        let options = DeviceOptions {
            poll_override,
            attributes: parse_attributes(&args.attributes),
            download_dir: args.download_dir.clone(),
            once: args.once,
            fail_download: args.fail_download,
            deny_confirmation: args.deny_confirmation,
            defer_soft: args.defer_soft,
            auto_confirm: true,
        };
        handles.push(tokio::spawn(async move {
            Device::new(client, options).run().await
        }));
    }

    for handle in handles {
        handle.await??;
    }
    Ok(())
}

/// Poll-only benchmark: N devices hammer the controller base resource;
/// aggregated stats are printed once a second as machine-parsable POLLSTAT
/// lines (consumed by scripts/load_poll.sh).
async fn poll_only_fleet(args: &Args, interval: Duration) -> anyhow::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    let total = Arc::new(AtomicU64::new(0));
    let errors = Arc::new(AtomicU64::new(0));
    let latency_us_sum = Arc::new(AtomicU64::new(0));
    let latency_us_max = Arc::new(AtomicU64::new(0));

    for n in 0..args.count {
        let controller_id = if args.count == 1 {
            args.controller_id.clone()
        } else {
            format!("{}-{}", args.controller_id, n)
        };
        let auth = match (&args.token, &args.gateway_token) {
            (Some(t), _) => DdiAuth::TargetToken(t.clone()),
            (None, Some(g)) => DdiAuth::GatewayToken(g.clone()),
            (None, None) => DdiAuth::None,
        };
        let client = DdiClient::new(args.url.clone(), args.tenant.clone(), controller_id, auth);
        let (total, errors, sum, max) = (
            total.clone(),
            errors.clone(),
            latency_us_sum.clone(),
            latency_us_max.clone(),
        );
        tokio::spawn(async move {
            // Spread the fleet across the interval to avoid a thundering herd.
            tokio::time::sleep(interval.mul_f64(fastrand_like(n))).await;
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                let started = std::time::Instant::now();
                match client.poll().await {
                    Ok(_) => {
                        let us = started.elapsed().as_micros() as u64;
                        total.fetch_add(1, Ordering::Relaxed);
                        sum.fetch_add(us, Ordering::Relaxed);
                        max.fetch_max(us, Ordering::Relaxed);
                    }
                    Err(_) => {
                        errors.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        });
    }

    // Reporter: one POLLSTAT line per second until killed by the harness.
    let started = std::time::Instant::now();
    let mut last_total = 0u64;
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    loop {
        ticker.tick().await;
        let now_total = total.load(Ordering::Relaxed);
        let now_errors = errors.load(Ordering::Relaxed);
        let sum = latency_us_sum.load(Ordering::Relaxed);
        let max = latency_us_max.load(Ordering::Relaxed);
        let rps = now_total - last_total;
        last_total = now_total;
        let avg_ms = if now_total > 0 {
            sum as f64 / now_total as f64 / 1000.0
        } else {
            0.0
        };
        println!(
            "POLLSTAT elapsed={:.0} total={now_total} errors={now_errors} rps={rps} avg_ms={avg_ms:.2} max_ms={:.1}",
            started.elapsed().as_secs_f64(),
            max as f64 / 1000.0,
        );
    }
}

/// Cheap deterministic jitter in [0,1) without a rand dependency here.
fn fastrand_like(n: usize) -> f64 {
    let h = (n as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    (h >> 40) as f64 / (1u64 << 24) as f64
}
