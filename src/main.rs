mod aur;
mod config;

use std::{fs, path::PathBuf, time::Duration};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde::Serialize;

#[derive(Parser)]
#[command(version, about = "Track your AUR packages: votes, popularity, downloads, views")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Fetch current stats, save them to the state dir, and print them.
    Fetch {
        /// Print JSON instead of a table.
        #[arg(long)]
        json: bool,
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
}

#[derive(Debug, Serialize)]
struct Snapshot {
    fetched_at: i64,
    maintainer: String,
    packages: Vec<PackageStats>,
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

fn state_dir() -> Result<PathBuf> {
    Ok(dirs::state_dir().context("no XDG state directory")?.join("aurwatch"))
}

fn fetch(cfg: &config::Config) -> Result<Snapshot> {
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

    let packages = pkgs
        .into_iter()
        .map(|p| PackageStats {
            github_repo: cfg
                .repos
                .get(&p.name)
                .or_else(|| cfg.repos.get(&p.package_base))
                .cloned()
                .or_else(|| p.url.as_deref().and_then(infer_repo)),
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
        })
        .collect();

    Ok(Snapshot {
        fetched_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs() as i64,
        maintainer: cfg.maintainer.clone(),
        packages,
    })
}

fn print_table(s: &Snapshot) {
    println!(
        "{:<18} {:<12} {:>5} {:>10}  {:<8} GITHUB",
        "PACKAGE", "VERSION", "VOTES", "POPULARITY", "FLAGGED"
    );
    for p in &s.packages {
        println!(
            "{:<18} {:<12} {:>5} {:>10.4}  {:<8} {}",
            p.name,
            p.version,
            p.votes,
            p.popularity,
            if p.out_of_date_since.is_some() { "yes" } else { "-" },
            p.github_repo.as_deref().unwrap_or("?"),
        );
    }
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Config => println!("{}", config::path()?.display()),
        Cmd::Fetch { json } => {
            let cfg = config::load()?;
            let snap = fetch(&cfg)?;
            let out = serde_json::to_string_pretty(&snap)?;

            // latest.json is what the TUI and bar widget will read.
            let dir = state_dir()?;
            fs::create_dir_all(&dir)?;
            let tmp = dir.join("latest.json.tmp");
            fs::write(&tmp, &out)?;
            fs::rename(&tmp, dir.join("latest.json"))?;

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
    use super::infer_repo;

    #[test]
    fn infers_github_repos() {
        assert_eq!(
            infer_repo("https://github.com/cybercoretech/dwcode").as_deref(),
            Some("cybercoretech/dwcode")
        );
        assert_eq!(infer_repo("https://github.com/o/r.git").as_deref(), Some("o/r"));
        assert_eq!(
            infer_repo("https://cybercoretech.github.io/cyberterm/").as_deref(),
            Some("cybercoretech/cyberterm")
        );
        assert_eq!(infer_repo("https://example.com/x"), None);
        assert_eq!(infer_repo("https://cybercoretech.github.io/"), None);
    }
}
