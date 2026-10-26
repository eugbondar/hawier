// Shared by every binary crate's `build.rs` (via `include!`): stamps one
// build string, `vYY.MM.DD-<short sha>`, into `HAWIER_BUILD`.
//
// The sha comes from `GIT_SHA` when the build happens in a container that
// has no `.git`, otherwise from git itself, otherwise `nogit`.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn from_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// `YY.MM.DD` for the current day (civil date from the Unix epoch).
fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{:02}.{:02}.{:02}", y % 100, m, d)
}

fn emit_build_version() {
    println!("cargo:rerun-if-env-changed=GIT_SHA");
    println!("cargo:rerun-if-env-changed=BUILD_DATE");
    // Only when it is really there. A `rerun-if-changed` naming a path that
    // does not exist is treated by cargo as "changed", every time — and the
    // container build has no `.git` at all (it is dockerignored; that is why
    // GIT_SHA is passed in). So this line, meant to catch a new commit, was
    // instead rerunning all three build scripts on every single build,
    // re-emitting HAWIER_BUILD, and forcing a recompile and a thin-LTO
    // relink of all three binaries even when nothing had changed.
    if std::path::Path::new("../../.git/HEAD").exists() {
        println!("cargo:rerun-if-changed=../../.git/HEAD");
    }

    let sha = from_env("GIT_SHA")
        .or_else(|| git(&["rev-parse", "--short=9", "HEAD"]))
        .unwrap_or_else(|| "nogit".to_string());
    let date = from_env("BUILD_DATE")
        .or_else(|| git(&["log", "-1", "--format=%cd", "--date=format:%y.%m.%d"]))
        .unwrap_or_else(today);
    println!("cargo:rustc-env=HAWIER_BUILD=v{date}-{sha}");
}
