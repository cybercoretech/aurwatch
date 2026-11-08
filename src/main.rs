mod aur;
mod config;
mod github;
mod history;
mod snapshot;
mod tui;

use anyhow::Result;
use clap::{Parser, Subcommand};

use snapshot::Snapshot;

#[derive(Parser)]
#[command(
    version,
    about = "Track your AUR packages: votes, popularity, downloads, views",
    long_about = "Track your AUR packages: votes, popularity, downloads, views.\n\nWith no command, opens the terminal UI."
)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Open the terminal UI (the default).
    Tui,
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
    for p in &s.packages {
        if let Some(tag) = p.unpackaged_release() {
            println!(
                "note: {}: GitHub release {tag} is newer than the AUR's {}",
                p.name, p.version
            );
        }
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
    match Cli::parse().cmd.unwrap_or(Cmd::Tui) {
        Cmd::Tui => tui::run()?,
        Cmd::Config => println!("{}", config::path()?.display()),
        Cmd::Fetch { json, no_github } => {
            let (snap, _) = snapshot::fetch_and_save(!no_github)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&snap)?);
            } else {
                print_table(&snap);
            }
        }
    }
    Ok(())
}
