//! GitHub stats over the REST API, authenticated with the user's existing `gh` login.
//!
//! - Release asset `download_count`: the closest thing to AUR downloads, but
//!   only for `-bin` packages, whose PKGBUILD fetches a release asset. Source
//!   packages fetch tag archives, which GitHub doesn't count.
//! - Traffic (views/clones): GitHub repo traffic, not AUR page views. Needs
//!   push access, and covers only the last 14 days, so `history` keeps the
//!   per-day breakdown to build longer history.
//!
//! The token comes from `GH_TOKEN`/`GITHUB_TOKEN`, else one `gh auth token`
//! call. Requests then go straight to api.github.com: one `gh` subprocess per
//! request was slow, and parallel runs of a wrapped `gh` (mise shims) could hang.

use std::{
    io::Read,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

const API: &str = "https://api.github.com";
const TOKEN_TIMEOUT: Duration = Duration::from_secs(15);

/// A GitHub token: env var first, then `gh auth token` (killed after 15s).
pub fn token() -> Result<String> {
    for var in ["GH_TOKEN", "GITHUB_TOKEN"] {
        if let Ok(t) = std::env::var(var)
            && !t.trim().is_empty()
        {
            return Ok(t.trim().to_string());
        }
    }
    let mut child = Command::new("gh")
        .args(["auth", "token"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("running `gh auth token` (is the GitHub CLI installed?)")?;
    let start = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if start.elapsed() > TOKEN_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "`gh auth token` timed out after {}s",
                TOKEN_TIMEOUT.as_secs()
            );
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    let mut out = String::new();
    child.stdout.take().map(|mut o| o.read_to_string(&mut out));
    if !status.success() || out.trim().is_empty() {
        let mut err = String::new();
        child.stderr.take().map(|mut e| e.read_to_string(&mut err));
        bail!("`gh auth token` failed: {}", err.trim());
    }
    Ok(out.trim().to_string())
}

fn api<T: DeserializeOwned>(agent: &ureq::Agent, token: &str, path: &str) -> Result<T> {
    let resp = agent
        .get(&format!("{API}/{path}"))
        .header("Authorization", &format!("Bearer {token}"))
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .call();
    match resp {
        Ok(mut r) => r
            .body_mut()
            .read_json()
            .with_context(|| format!("decoding {path}")),
        Err(ureq::Error::StatusCode(code)) => Err(anyhow!("{path}: HTTP {code}")),
        Err(e) => Err(anyhow!("{path}: {e}")),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    pub name: String,
    pub download_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Release {
    pub tag_name: String,
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    pub assets: Vec<Asset>,
}

/// Newest published, non-prerelease tag (GitHub lists releases newest first).
pub fn latest_tag(releases: &[Release]) -> Option<&str> {
    releases
        .iter()
        .find(|r| !r.prerelease)
        .map(|r| r.tag_name.as_str())
}

/// `v1.2.3` / `cli-v1.2.3` / `1.2.3` -> `1.2.3`
pub fn tag_version(tag: &str) -> &str {
    tag.find(|c: char| c.is_ascii_digit())
        .map_or(tag, |i| &tag[i..])
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrafficDay {
    /// ISO 8601, midnight UTC, e.g. `2026-11-01T00:00:00Z`.
    pub timestamp: String,
    pub count: u64,
    pub uniques: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Traffic {
    /// Totals over GitHub's 14-day window.
    pub count: u64,
    pub uniques: u64,
    pub days: Vec<TrafficDay>,
}

#[derive(Deserialize)]
struct ViewsResp {
    count: u64,
    uniques: u64,
    views: Vec<TrafficDay>,
}

#[derive(Deserialize)]
struct ClonesResp {
    count: u64,
    uniques: u64,
    clones: Vec<TrafficDay>,
}

/// Everything aurwatch knows about one GitHub repo. Each part fails independently
/// so a missing traffic permission doesn't hide release downloads.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RepoStats {
    pub releases: Option<Vec<Release>>,
    pub views: Option<Traffic>,
    pub clones: Option<Traffic>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

pub fn repo_stats(agent: &ureq::Agent, token: &str, repo: &str) -> RepoStats {
    let mut s = RepoStats::default();
    match api::<Vec<Release>>(agent, token, &format!("repos/{repo}/releases?per_page=100")) {
        Ok(r) => s.releases = Some(r.into_iter().filter(|r| !r.draft).collect()),
        Err(e) => s.errors.push(e.to_string()),
    }
    match api::<ViewsResp>(agent, token, &format!("repos/{repo}/traffic/views")) {
        Ok(v) => {
            s.views = Some(Traffic {
                count: v.count,
                uniques: v.uniques,
                days: v.views,
            })
        }
        Err(e) => s.errors.push(e.to_string()),
    }
    match api::<ClonesResp>(agent, token, &format!("repos/{repo}/traffic/clones")) {
        Ok(c) => {
            s.clones = Some(Traffic {
                count: c.count,
                uniques: c.uniques,
                days: c.clones,
            })
        }
        Err(e) => s.errors.push(e.to_string()),
    }
    s
}

/// Is this release asset something an AUR `-bin` package would install from?
/// Excludes checksums/signatures/metadata and non-Linux builds.
pub fn is_linux_artifact(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    const META: &[&str] = &[
        ".sha256", ".sha512", ".sig", ".asc", ".minisig", ".json", ".txt", ".cdx", ".pem", ".sbom",
        ".dwpack",
    ];
    const OTHER_OS: &[&str] = &[
        "darwin", "apple", "macos", "windows", "msvc", ".exe", ".apk", ".dmg",
    ];
    if META
        .iter()
        .any(|m| n.ends_with(m) || n.contains(&format!("{m}.")))
    {
        return false;
    }
    if OTHER_OS.iter().any(|o| n.contains(o)) {
        return false;
    }
    n.contains("linux") || n.contains("x86_64") || n.contains("aarch64") || n.contains("amd64")
}

/// Linux artifact downloads: (current release, all releases).
/// `current` matches the AUR pkgver against the tag (`v1.2.3` or `1.2.3`, ignoring prefixes like `cli-`).
pub fn linux_downloads(releases: &[Release], pkgver: &str) -> (Option<u64>, u64) {
    let sum = |r: &Release| -> u64 {
        r.assets
            .iter()
            .filter(|a| is_linux_artifact(&a.name))
            .map(|a| a.download_count)
            .sum()
    };
    let total = releases.iter().map(sum).sum();
    let current = releases
        .iter()
        .find(|r| {
            let t = r.tag_name.as_str();
            t == pkgver
                || t.strip_suffix(pkgver)
                    .is_some_and(|p| p.ends_with('v') || p.ends_with('-'))
        })
        .map(sum);
    (current, total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(tag: &str, assets: &[(&str, u64)]) -> Release {
        Release {
            tag_name: tag.into(),
            draft: false,
            prerelease: false,
            assets: assets
                .iter()
                .map(|(n, c)| Asset {
                    name: (*n).into(),
                    download_count: *c,
                })
                .collect(),
        }
    }

    #[test]
    fn filters_artifacts() {
        assert!(is_linux_artifact(
            "cyberterm-x86_64-unknown-linux-gnu.tar.gz"
        ));
        assert!(is_linux_artifact("dwcode-0.1.0-x86_64.tar.gz"));
        assert!(!is_linux_artifact(
            "cyberterm-x86_64-unknown-linux-gnu.tar.gz.sha256"
        ));
        assert!(!is_linux_artifact("cyberterm-aarch64-apple-darwin.tar.gz"));
        assert!(!is_linux_artifact(
            "agentforge-0.4.0-x86_64-pc-windows-msvc.tar.gz"
        ));
        assert!(!is_linux_artifact("index.json"));
        assert!(!is_linux_artifact("cybercore.dwcode-core-0.1.0.dwpack"));
        assert!(!is_linux_artifact(
            "hermes-v0.2.1-x86_64-unknown-linux-gnu.tar.gz.sigstore.json"
        ));
    }

    #[test]
    fn latest_and_tag_versions() {
        let mut pre = rel("v0.5.0-rc1", &[]);
        pre.prerelease = true;
        let rels = vec![pre, rel("cli-v0.4.1", &[]), rel("v0.4.0", &[])];
        assert_eq!(latest_tag(&rels), Some("cli-v0.4.1"));
        assert_eq!(latest_tag(&[]), None);
        assert_eq!(tag_version("cli-v0.4.1"), "0.4.1");
        assert_eq!(tag_version("v1.0"), "1.0");
        assert_eq!(tag_version("2.0.0"), "2.0.0");
    }

    #[test]
    fn sums_current_and_total() {
        let rels = vec![
            rel(
                "v0.4.1",
                &[
                    ("c-x86_64-unknown-linux-gnu.tar.gz", 22),
                    ("c-aarch64-apple-darwin.tar.gz", 11),
                    ("c-aarch64-unknown-linux-gnu.tar.gz", 4),
                ],
            ),
            rel("v0.4.0", &[("c-x86_64-unknown-linux-gnu.tar.gz", 2)]),
        ];
        assert_eq!(linux_downloads(&rels, "0.4.1"), (Some(26), 28));
        assert_eq!(linux_downloads(&rels, "0.4.0"), (Some(2), 28));
        assert_eq!(linux_downloads(&rels, "0.5.0"), (None, 28));
        // 0.4.1 must not match a v10.4.1 tag
        assert_eq!(
            linux_downloads(&[rel("v10.4.1", &[("x86_64.tar.gz", 1)])], "0.4.1").0,
            None
        );
        assert_eq!(
            linux_downloads(&[rel("cli-v0.8.0", &[("x86_64.tar.gz", 3)])], "0.8.0").0,
            Some(3)
        );
    }
}
