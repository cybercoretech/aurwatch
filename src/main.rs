mod aur;
mod config;
mod github;
mod history;

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde::Serialize;

use history::{DailyRow, History};

#[derive(Parser)]
#[command(
    version,
    about = "Track your AUR packages: votes, popularity, downloads, views"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Fetch current stats, update history, save latest.json, and print them.
    Fetch {
        /// Print JSON instead of a table.
        #[arg(long)]
        json: bool,
        /// AUR data only; skip GitHub downloads and traffic.
        #[arg(long)]
        no_github: bool,
    },
    /// Print the config file path.
    Config,
}

#[derive(Debug, Serialize)]
struct PackageStats {
    name: String,
    base: String,
    version: String,
    description: Option<String>,
    /// Differs from the snapshot's maintainer for `extra_packages`; `None` means orphaned.
    maintainer: Option<String>,
    votes: u32,
    popularity: f64,
    out_of_date_since: Option<i64>,
    first_submitted: i64,
    last_modified: i64,
    aur_url: String,
    upstream_url: Option<String>,
    /// GitHub `owner/repo`, from config or inferred from the package URL.
    github_repo: Option<String>,
    /// Release-asset download counts. `-bin` packages only: source packages
    /// fetch tag archives, which GitHub doesn't count.
    downloads: Option<Downloads>,
}

#[derive(Debug, Serialize)]
struct Downloads {
    /// Linux assets of the release matching the AUR version.
    current_release: Option<u64>,
    /// Linux assets across all releases.
    all_releases: u64,
    /// From aurwatch's own history; `None` until it reaches back far enough.
    last_7d: Option<u64>,
    last_30d: Option<u64>,
}

/// GitHub repo traffic. This is the repo's views and clones, not the AUR page's.
#[derive(Debug, Serialize)]
struct RepoSummary {
    views_14d: Option<u64>,
    view_uniques_14d: Option<u64>,
    clones_14d: Option<u64>,
    clone_uniques_14d: Option<u64>,
    /// Totals across all history aurwatch has recorded.
    views_recorded: u64,
    clones_recorded: u64,
    days_recorded: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    errors: Vec<String>,
}

#[derive(Debug, Serialize)]
struct Snapshot {
    fetched_at: i64,
    maintainer: String,
    packages: Vec<PackageStats>,
    repos: BTreeMap<String, RepoSummary>,
}

/// `https://github.com/o/r[...]` or `https://o.github.io/r[...]` -> `o/r`.
fn infer_repo(url: &str) -> Option<String> {
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
fn pkgver(version: &str) -> &str {
    let v = version.split_once(':').map_or(version, |(_, v)| v);
    v.rsplit_once('-').map_or(v, |(v, _)| v)
}

fn state_dir() -> Result<PathBuf> {
    Ok(dirs::state_dir()
        .context("no XDG state directory")?
        .join("aurwatch"))
}

pub fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, contents).with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))
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

    // One `gh` round per unique repo, in parallel.
    let repos: BTreeSet<String> = pkgs.iter().filter_map(repo_of).collect();
    let gh: BTreeMap<String, github::RepoStats> = if with_github {
        std::thread::scope(|s| {
            let handles: Vec<_> = repos
                .iter()
                .map(|r| (r.clone(), s.spawn(move || github::repo_stats(r))))
                .collect();
            handles
                .into_iter()
                .map(|(r, h)| (r, h.join().unwrap_or_default()))
                .collect()
        })
    } else {
        BTreeMap::new()
    };

    let fetched_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs() as i64;
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

fn opt(n: Option<u64>) -> String {
    n.map_or_else(|| "-".into(), |n| n.to_string())
}

fn print_table(s: &Snapshot) {
    println!(
        "{:<16} {:<10} {:>5} {:>7}  {:>8} {:>8} {:>6} {:>6}  {:<7} GITHUB",
        "PACKAGE", "VERSION", "VOTES", "POP", "DL CUR", "DL ALL", "DL 7D", "DL 30D", "FLAGGED"
    );
    for p in &s.packages {
        let d = p.downloads.as_ref();
        println!(
            "{:<16} {:<10} {:>5} {:>7.4}  {:>8} {:>8} {:>6} {:>6}  {:<7} {}",
            p.name,
            p.version,
            p.votes,
            p.popularity,
            opt(d.and_then(|d| d.current_release)),
            opt(d.map(|d| d.all_releases)),
            opt(d.and_then(|d| d.last_7d)),
            opt(d.and_then(|d| d.last_30d)),
            if p.out_of_date_since.is_some() {
                "yes"
            } else {
                "-"
            },
            p.github_repo.as_deref().unwrap_or("?"),
        );
    }
    if s.repos.is_empty() {
        return;
    }
    println!(
        "\n{:<30} {:>9} {:>9} {:>10} {:>10} {:>9} {:>10} {:>5}",
        "GITHUB REPO",
        "VIEWS 14D",
        "UNIQUE",
        "CLONES 14D",
        "UNIQUE",
        "VIEWS ALL",
        "CLONES ALL",
        "DAYS"
    );
    for (repo, r) in &s.repos {
        println!(
            "{:<30} {:>9} {:>9} {:>10} {:>10} {:>9} {:>10} {:>5}",
            repo,
            opt(r.views_14d),
            opt(r.view_uniques_14d),
            opt(r.clones_14d),
            opt(r.clone_uniques_14d),
            r.views_recorded,
            r.clones_recorded,
            r.days_recorded,
        );
    }
    for (repo, r) in &s.repos {
        for e in &r.errors {
            eprintln!("warning: {repo}: {e}");
        }
    }
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Config => println!("{}", config::path()?.display()),
        Cmd::Fetch { json, no_github } => {
            let cfg = config::load()?;
            let dir = state_dir()?;
            fs::create_dir_all(&dir)?;
            let hist_path = dir.join("history.json");
            let mut hist = History::load(&hist_path)?;

            let snap = fetch(&cfg, !no_github, &mut hist)?;
            let out = serde_json::to_string_pretty(&snap)?;

            hist.save(&hist_path)?;
            // latest.json is what the TUI and bar widget read.
            write_atomic(&dir.join("latest.json"), &out)?;

            if json {
                println!("{out}")
            } else {
                print_table(&snap)
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{infer_repo, pkgver};

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
}
