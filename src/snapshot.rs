//! One fetch of everything aurwatch tracks, plus where it lives on disk.
//! `latest.json` holds the newest `Snapshot`; the TUI and bar widget read it.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::{
    aur, config, github,
    history::{self, DailyRow, History},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageStats {
    pub name: String,
    pub base: String,
    pub version: String,
    pub description: Option<String>,
    /// Differs from the snapshot's maintainer for `extra_packages`; `None` means orphaned.
    pub maintainer: Option<String>,
    pub votes: u32,
    pub popularity: f64,
    pub out_of_date_since: Option<i64>,
    pub first_submitted: i64,
    pub last_modified: i64,
    pub aur_url: String,
    pub upstream_url: Option<String>,
    /// GitHub `owner/repo`, from config or inferred from the package URL.
    pub github_repo: Option<String>,
    /// Release-asset download counts. `-bin` packages only: source packages
    /// fetch tag archives, which GitHub doesn't count.
    pub downloads: Option<Downloads>,
    /// Newest non-prerelease GitHub release tag of the package's repo.
    #[serde(default)]
    pub latest_release: Option<String>,
}

impl PackageStats {
    /// The latest GitHub release, when its version differs from what the AUR ships.
    pub fn unpackaged_release(&self) -> Option<&str> {
        let tag = self.latest_release.as_deref()?;
        (github::tag_version(tag) != pkgver(&self.version)).then_some(tag)
    }

    /// What needs the maintainer's attention, as short phrases. Empty means fine.
    pub fn alerts(&self) -> Vec<String> {
        let mut a = Vec::new();
        if self.out_of_date_since.is_some() {
            a.push("flagged out-of-date".into());
        }
        if let Some(tag) = self.unpackaged_release() {
            a.push(format!("{tag} not packaged"));
        }
        if self.maintainer.is_none() {
            a.push("orphaned".into());
        }
        a
    }
}

/// Snapshots older than this mean the timer (every 6h) missed two runs.
pub const STALE_SECS: i64 = 13 * 3600;

/// Compact, network-free digest of `latest.json` for the bar widget.
#[derive(Debug, Serialize)]
pub struct Summary {
    pub have_data: bool,
    pub fetched_at: i64,
    pub age_secs: i64,
    pub stale: bool,
    pub packages: usize,
    pub votes: u32,
    /// Linux release-asset downloads across `-bin` packages, all releases.
    pub downloads: u64,
    /// Sum over `-bin` packages that have 7 days of history; `None` before that.
    pub downloads_7d: Option<u64>,
    pub alerts: usize,
    /// One line per package, for the tooltip.
    pub lines: Vec<String>,
}

impl Summary {
    pub fn from_snapshot(snap: Option<&Snapshot>, now: i64) -> Self {
        let Some(s) = snap else {
            return Self {
                have_data: false,
                fetched_at: 0,
                age_secs: 0,
                stale: true,
                packages: 0,
                votes: 0,
                downloads: 0,
                downloads_7d: None,
                alerts: 0,
                lines: Vec::new(),
            };
        };
        let bins = || s.packages.iter().filter_map(|p| p.downloads.as_ref());
        let week: Vec<u64> = bins().filter_map(|d| d.last_7d).collect();
        let lines = s
            .packages
            .iter()
            .map(|p| {
                let mut l = format!("{}  {}  ★{}", p.name, p.version, p.votes);
                if let Some(d) = &p.downloads {
                    l.push_str(&format!("  ↓{}", d.all_releases));
                }
                let alerts = p.alerts();
                if !alerts.is_empty() {
                    l.push_str(&format!("  ⚠ {}", alerts.join(", ")));
                }
                l
            })
            .collect();
        let age_secs = (now - s.fetched_at).max(0);
        Self {
            have_data: true,
            fetched_at: s.fetched_at,
            age_secs,
            stale: age_secs > STALE_SECS,
            packages: s.packages.len(),
            votes: s.packages.iter().map(|p| p.votes).sum(),
            downloads: bins().map(|d| d.all_releases).sum(),
            downloads_7d: (!week.is_empty()).then(|| week.iter().sum()),
            alerts: s.packages.iter().filter(|p| !p.alerts().is_empty()).count(),
            lines,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Downloads {
    /// Linux assets of the release matching the AUR version.
    pub current_release: Option<u64>,
    /// Linux assets across all releases.
    pub all_releases: u64,
    /// From aurwatch's own history; `None` until it reaches back far enough.
    pub last_7d: Option<u64>,
    pub last_30d: Option<u64>,
}

/// GitHub repo traffic. This is the repo's views and clones, not the AUR page's.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoSummary {
    pub views_14d: Option<u64>,
    pub view_uniques_14d: Option<u64>,
    pub clones_14d: Option<u64>,
    pub clone_uniques_14d: Option<u64>,
    /// Totals across all history aurwatch has recorded.
    pub views_recorded: u64,
    pub clones_recorded: u64,
    pub days_recorded: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub fetched_at: i64,
    pub maintainer: String,
    pub packages: Vec<PackageStats>,
    pub repos: BTreeMap<String, RepoSummary>,
}

/// `https://github.com/o/r[...]` or `https://o.github.io/r[...]` -> `o/r`.
pub fn infer_repo(url: &str) -> Option<String> {
    let rest = url.split("://").nth(1)?;
    let mut parts = rest.split('/').filter(|s| !s.is_empty());
    let host = parts.next()?.to_ascii_lowercase();
    if host == "github.com" || host == "www.github.com" {
        let owner = parts.next()?;
        let repo = parts.next()?.trim_end_matches(".git");
        return Some(format!("{owner}/{repo}"));
    }
    let owner = host.strip_suffix(".github.io")?;
    Some(format!("{owner}/{}", parts.next()?))
}

/// `1:0.4.1-2` -> `0.4.1`
pub fn pkgver(version: &str) -> &str {
    let v = version.split_once(':').map_or(version, |(_, v)| v);
    v.rsplit_once('-').map_or(v, |(v, _)| v)
}

pub fn state_dir() -> Result<PathBuf> {
    Ok(dirs::state_dir()
        .context("no XDG state directory")?
        .join("aurwatch"))
}

pub fn latest_path() -> Result<PathBuf> {
    Ok(state_dir()?.join("latest.json"))
}

pub fn history_path() -> Result<PathBuf> {
    Ok(state_dir()?.join("history.json"))
}

pub fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, contents).with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// The last saved snapshot, if there is one.
pub fn load_latest() -> Result<Option<Snapshot>> {
    let path = latest_path()?;
    match fs::read_to_string(&path) {
        Ok(raw) => Ok(Some(
            serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?,
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// Fetch, merge into history, and save both `history.json` and `latest.json`.
pub fn fetch_and_save(with_github: bool) -> Result<(Snapshot, History)> {
    let cfg = config::load()?;
    fs::create_dir_all(state_dir()?)?;
    let hist_path = history_path()?;
    let mut hist = History::load(&hist_path)?;
    let snap = fetch(&cfg, with_github, &mut hist)?;
    hist.save(&hist_path)?;
    write_atomic(&latest_path()?, &serde_json::to_string_pretty(&snap)?)?;
    Ok((snap, hist))
}

fn fetch(cfg: &config::Config, with_github: bool, hist: &mut History) -> Result<Snapshot> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .user_agent(concat!("aurwatch/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();

    let mut pkgs = aur::by_maintainer(&agent, &cfg.maintainer)?;
    let missing: Vec<String> = cfg
        .extra_packages
        .iter()
        .filter(|n| !pkgs.iter().any(|p| &p.name == *n))
        .cloned()
        .collect();
    pkgs.extend(aur::info(&agent, &missing)?);
    pkgs.sort_by(|a, b| a.name.cmp(&b.name));

    let repo_of = |p: &aur::Package| {
        cfg.repos
            .get(&p.name)
            .or_else(|| cfg.repos.get(&p.package_base))
            .cloned()
            .or_else(|| p.url.as_deref().and_then(infer_repo))
    };

    // One token lookup, then each unique repo's requests in parallel.
    let repos: BTreeSet<String> = pkgs.iter().filter_map(repo_of).collect();
    let gh: BTreeMap<String, github::RepoStats> = match with_github.then(github::token) {
        None => BTreeMap::new(),
        // Without a token, report it on every repo instead of failing the AUR fetch.
        Some(Err(e)) => repos
            .iter()
            .map(|r| {
                let errors = vec![format!("no GitHub token: {e:#}")];
                (
                    r.clone(),
                    github::RepoStats {
                        errors,
                        ..Default::default()
                    },
                )
            })
            .collect(),
        Some(Ok(token)) => std::thread::scope(|s| {
            let (agent, token) = (&agent, token.as_str());
            let handles: Vec<_> = repos
                .iter()
                .map(|r| {
                    (
                        r.clone(),
                        s.spawn(move || github::repo_stats(agent, token, r)),
                    )
                })
                .collect();
            handles
                .into_iter()
                .map(|(r, h)| (r, h.join().unwrap_or_default()))
                .collect()
        }),
    };

    let fetched_at = now();
    let today = history::date_from_unix(fetched_at);

    let mut packages = Vec::with_capacity(pkgs.len());
    for p in pkgs {
        let github_repo = repo_of(&p);
        let releases = github_repo
            .as_ref()
            .and_then(|r| gh.get(r))
            .and_then(|s| s.releases.as_deref());
        let downloads = match releases {
            Some(rels) if p.name.ends_with("-bin") => {
                let (current_release, all_releases) =
                    github::linux_downloads(rels, pkgver(&p.version));
                Some(Downloads {
                    current_release,
                    all_releases,
                    last_7d: None,
                    last_30d: None,
                })
            }
            _ => None,
        };
        hist.record_daily(
            &p.name,
            &today,
            DailyRow {
                votes: p.num_votes,
                popularity: p.popularity,
                downloads_total: downloads.as_ref().map(|d| d.all_releases),
            },
        );
        let downloads = downloads.map(|d| Downloads {
            last_7d: hist.downloads_since(&p.name, &today, 7),
            last_30d: hist.downloads_since(&p.name, &today, 30),
            ..d
        });
        packages.push(PackageStats {
            aur_url: aur::page_url(&p.name),
            latest_release: releases.and_then(github::latest_tag).map(str::to_string),
            name: p.name,
            base: p.package_base,
            version: p.version,
            description: p.description,
            maintainer: p.maintainer,
            votes: p.num_votes,
            popularity: p.popularity,
            out_of_date_since: p.out_of_date,
            first_submitted: p.first_submitted,
            last_modified: p.last_modified,
            upstream_url: p.url,
            github_repo,
            downloads,
        });
    }

    let mut repo_summaries = BTreeMap::new();
    for (repo, s) in &gh {
        hist.merge_traffic(repo, s.views.as_ref(), s.clones.as_ref());
        let (views_recorded, clones_recorded, days_recorded) = hist.traffic_totals(repo);
        repo_summaries.insert(
            repo.clone(),
            RepoSummary {
                views_14d: s.views.as_ref().map(|t| t.count),
                view_uniques_14d: s.views.as_ref().map(|t| t.uniques),
                clones_14d: s.clones.as_ref().map(|t| t.count),
                clone_uniques_14d: s.clones.as_ref().map(|t| t.uniques),
                views_recorded,
                clones_recorded,
                days_recorded,
                errors: s.errors.clone(),
            },
        );
    }

    Ok(Snapshot {
        fetched_at,
        maintainer: cfg.maintainer.clone(),
        packages,
        repos: repo_summaries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infers_github_repos() {
        assert_eq!(
            infer_repo("https://github.com/cybercoretech/dwcode").as_deref(),
            Some("cybercoretech/dwcode")
        );
        assert_eq!(
            infer_repo("https://github.com/o/r.git").as_deref(),
            Some("o/r")
        );
        assert_eq!(
            infer_repo("https://cybercoretech.github.io/cyberterm/").as_deref(),
            Some("cybercoretech/cyberterm")
        );
        assert_eq!(infer_repo("https://example.com/x"), None);
        assert_eq!(infer_repo("https://cybercoretech.github.io/"), None);
    }

    #[test]
    fn strips_pkgrel_and_epoch() {
        assert_eq!(pkgver("0.4.1-1"), "0.4.1");
        assert_eq!(pkgver("1:2.0-3"), "2.0");
        assert_eq!(pkgver("0.1.0"), "0.1.0");
    }

    fn pkg(
        name: &str,
        version: &str,
        dl: Option<(u64, Option<u64>)>,
        tag: Option<&str>,
    ) -> PackageStats {
        PackageStats {
            name: name.into(),
            base: name.into(),
            version: version.into(),
            description: None,
            maintainer: Some("me".into()),
            votes: 2,
            popularity: 0.0,
            out_of_date_since: None,
            first_submitted: 0,
            last_modified: 0,
            aur_url: String::new(),
            upstream_url: None,
            github_repo: None,
            downloads: dl.map(|(all, week)| Downloads {
                current_release: Some(all),
                all_releases: all,
                last_7d: week,
                last_30d: None,
            }),
            latest_release: tag.map(str::to_string),
        }
    }

    #[test]
    fn summary_digest() {
        let empty = Summary::from_snapshot(None, 100);
        assert!(!empty.have_data && empty.stale && empty.lines.is_empty());

        let mut orphan = pkg("old", "1.0-1", None, None);
        orphan.maintainer = None;
        let snap = Snapshot {
            fetched_at: 1000,
            maintainer: "me".into(),
            packages: vec![
                pkg("a", "0.4.1-1", None, Some("v0.4.1")),
                pkg("a-bin", "0.4.1-1", Some((30, Some(4))), Some("v0.5.0")),
                pkg("b-bin", "0.1.0-1", Some((20, None)), None),
                orphan,
            ],
            repos: BTreeMap::new(),
        };
        let s = Summary::from_snapshot(Some(&snap), 1000 + 3600);
        assert_eq!(
            (s.packages, s.votes, s.downloads, s.downloads_7d),
            (4, 8, 50, Some(4))
        );
        assert_eq!(s.alerts, 2); // a-bin (v0.5.0 not packaged) and old (orphaned)
        assert!(!s.stale && s.age_secs == 3600);
        assert_eq!(s.lines[1], "a-bin  0.4.1-1  ★2  ↓30  ⚠ v0.5.0 not packaged");
        assert_eq!(s.lines[0], "a  0.4.1-1  ★2");
        assert!(Summary::from_snapshot(Some(&snap), 1000 + STALE_SECS + 1).stale);
    }
}
