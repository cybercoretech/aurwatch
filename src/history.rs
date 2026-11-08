//! `~/.local/state/aurwatch/history.json`: long-term history.
//!
//! Two kinds of data, both keyed by UTC date (`YYYY-MM-DD`):
//! - `traffic`: GitHub's per-day views/clones. Each fetch merges the 14-day
//!   window in, newest value winning, so history grows past 14 days as long as
//!   aurwatch runs at least once every two weeks.
//! - `daily`: one row per package per day of cumulative numbers (votes,
//!   popularity, total downloads). The last fetch of a day overwrites earlier
//!   ones; per-period downloads are differences between rows.

use std::{collections::BTreeMap, fs, path::Path};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::github::Traffic;

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct TrafficRow {
    pub views: u64,
    pub view_uniques: u64,
    pub clones: u64,
    pub clone_uniques: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct DailyRow {
    pub votes: u32,
    pub popularity: f64,
    /// Cumulative Linux release-asset downloads (bin packages only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub downloads_total: Option<u64>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct History {
    /// repo -> date -> traffic
    #[serde(default)]
    pub traffic: BTreeMap<String, BTreeMap<String, TrafficRow>>,
    /// package -> date -> numbers
    #[serde(default)]
    pub daily: BTreeMap<String, BTreeMap<String, DailyRow>>,
}

impl History {
    pub fn load(path: &Path) -> Result<Self> {
        match fs::read_to_string(path) {
            Ok(raw) => {
                serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        crate::write_atomic(path, &serde_json::to_string_pretty(self)?)
    }

    /// Merge GitHub's 14-day views and clones windows for one repo.
    pub fn merge_traffic(&mut self, repo: &str, views: Option<&Traffic>, clones: Option<&Traffic>) {
        let days = self.traffic.entry(repo.to_string()).or_default();
        for d in views.into_iter().flat_map(|t| &t.days) {
            let row = days.entry(date_of(&d.timestamp)).or_default();
            row.views = d.count;
            row.view_uniques = d.uniques;
        }
        for d in clones.into_iter().flat_map(|t| &t.days) {
            let row = days.entry(date_of(&d.timestamp)).or_default();
            row.clones = d.count;
            row.clone_uniques = d.uniques;
        }
    }

    /// Record today's row. A fetch without download data (`--no-github`, or a
    /// GitHub error) keeps the download total already recorded for that day.
    pub fn record_daily(&mut self, package: &str, date: &str, mut row: DailyRow) {
        let days = self.daily.entry(package.to_string()).or_default();
        if row.downloads_total.is_none() {
            row.downloads_total = days.get(date).and_then(|r| r.downloads_total);
        }
        days.insert(date.to_string(), row);
    }

    /// Downloads in the last `days` days: latest total minus the total on the
    /// newest row at least `days` days old. `None` until history reaches back that far.
    pub fn downloads_since(&self, package: &str, today: &str, days: i64) -> Option<u64> {
        let rows = self.daily.get(package)?;
        let latest = rows
            .range(..=today.to_string())
            .next_back()?
            .1
            .downloads_total?;
        let cutoff = shift_date(today, -days)?;
        let base = rows.range(..=cutoff).next_back()?.1.downloads_total?;
        Some(latest.saturating_sub(base))
    }

    /// Sum of views and clones over all recorded days for a repo.
    pub fn traffic_totals(&self, repo: &str) -> (u64, u64, usize) {
        self.traffic.get(repo).map_or((0, 0, 0), |d| {
            (
                d.values().map(|r| r.views).sum(),
                d.values().map(|r| r.clones).sum(),
                d.len(),
            )
        })
    }
}

fn date_of(ts: &str) -> String {
    ts.get(..10).unwrap_or(ts).to_string()
}

/// Days since 1970-01-01 -> (y, m, d). Howard Hinnant's civil_from_days.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// (y, m, d) -> days since 1970-01-01. Inverse of `civil_from_days`.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = i64::from(if m > 2 { m - 3 } else { m + 9 });
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub fn date_from_unix(secs: i64) -> String {
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

fn shift_date(date: &str, days: i64) -> Option<String> {
    let mut it = date.splitn(3, '-');
    let (y, m, d) = (
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
    );
    Some(date_from_unix((days_from_civil(y, m, d) + days) * 86_400))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::TrafficDay;

    #[test]
    fn dates() {
        assert_eq!(date_from_unix(0), "1970-01-01");
        assert_eq!(date_from_unix(1_794_115_792), "2026-11-08"); // 05:29 UTC
        assert_eq!(date_from_unix(1_794_095_999), "2026-11-07"); // 23:59:59 UTC
        assert_eq!(shift_date("2026-03-01", -1).as_deref(), Some("2026-02-28"));
        assert_eq!(shift_date("2028-03-01", -1).as_deref(), Some("2028-02-29"));
        assert_eq!(shift_date("2026-11-07", -7).as_deref(), Some("2026-10-31"));
    }

    fn traffic(days: &[(&str, u64)]) -> Traffic {
        Traffic {
            count: days.iter().map(|d| d.1).sum(),
            uniques: 0,
            days: days
                .iter()
                .map(|(t, c)| TrafficDay {
                    timestamp: format!("{t}T00:00:00Z"),
                    count: *c,
                    uniques: 1,
                })
                .collect(),
        }
    }

    #[test]
    fn traffic_accumulates_past_window() {
        let mut h = History::default();
        h.merge_traffic(
            "o/r",
            Some(&traffic(&[("2026-10-01", 3), ("2026-10-02", 1)])),
            None,
        );
        // later window: 10-02 grew (day was partial), 10-03 is new, 10-01 dropped out of GitHub's window
        h.merge_traffic(
            "o/r",
            Some(&traffic(&[("2026-10-02", 4), ("2026-10-03", 2)])),
            Some(&traffic(&[("2026-10-03", 7)])),
        );
        assert_eq!(h.traffic_totals("o/r"), (3 + 4 + 2, 7, 3));
    }

    #[test]
    fn downloads_window() {
        let mut h = History::default();
        let row = |n| DailyRow {
            votes: 0,
            popularity: 0.0,
            downloads_total: Some(n),
        };
        h.record_daily("p", "2026-10-30", row(10));
        assert_eq!(h.downloads_since("p", "2026-10-30", 7), None);
        h.record_daily("p", "2026-11-01", row(15));
        h.record_daily("p", "2026-11-07", row(30));
        // newest row at or before 10-31 is 10-30 (10)
        assert_eq!(h.downloads_since("p", "2026-11-07", 7), Some(20));
        assert_eq!(h.downloads_since("p", "2026-11-07", 6), Some(15));
        // a later same-day fetch without GitHub data keeps the total, updates votes
        h.record_daily(
            "p",
            "2026-11-07",
            DailyRow {
                votes: 2,
                popularity: 0.1,
                downloads_total: None,
            },
        );
        assert_eq!(
            h.daily["p"]["2026-11-07"],
            DailyRow {
                votes: 2,
                popularity: 0.1,
                downloads_total: Some(30)
            }
        );
    }
}
