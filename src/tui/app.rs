//! TUI state: the loaded snapshot and history, selection, and background refresh.

use std::{
    process::{Command, Stdio},
    sync::mpsc::{self, Receiver, TryRecvError},
};

use anyhow::Result;
use ratatui::widgets::TableState;

use crate::{
    history::History,
    snapshot::{self, PackageStats, Snapshot},
};

/// Refresh on startup when the saved snapshot is older than this.
const STALE_SECS: i64 = 6 * 3600;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    List,
    Detail,
}

pub struct App {
    pub snap: Option<Snapshot>,
    pub hist: History,
    pub table: TableState,
    pub view: View,
    pub help: bool,
    pub status: String,
    pub status_is_error: bool,
    pub quit: bool,
    pub tick: usize,
    refresh_rx: Option<Receiver<Result<(Snapshot, History)>>>,
}

impl App {
    pub fn new() -> Result<Self> {
        let snap = snapshot::load_latest()?;
        let hist = History::load(&snapshot::history_path()?)?;
        let mut app = Self {
            table: TableState::default().with_selected(snap.as_ref().map(|_| 0)),
            snap,
            hist,
            view: View::List,
            help: false,
            status: String::new(),
            status_is_error: false,
            quit: false,
            tick: 0,
            refresh_rx: None,
        };
        let stale = app
            .snap
            .as_ref()
            .is_none_or(|s| snapshot::now() - s.fetched_at > STALE_SECS);
        if stale {
            app.refresh();
        }
        Ok(app)
    }

    pub fn refreshing(&self) -> bool {
        self.refresh_rx.is_some()
    }

    pub fn packages(&self) -> &[PackageStats] {
        self.snap.as_ref().map_or(&[], |s| &s.packages)
    }

    pub fn selected(&self) -> Option<&PackageStats> {
        self.packages().get(self.table.selected()?)
    }

    pub fn refresh(&mut self) {
        if self.refreshing() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(snapshot::fetch_and_save(true));
        });
        self.refresh_rx = Some(rx);
        self.set_status("refreshing…", false);
    }

    /// Called every loop iteration: animates the spinner and collects a finished refresh.
    pub fn poll(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        let Some(rx) = &self.refresh_rx else { return };
        let result = match rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(anyhow::anyhow!("refresh thread died")),
        };
        self.refresh_rx = None;
        match result {
            Ok((snap, hist)) => {
                let keep = self.selected().map(|p| p.name.clone());
                let idx = keep
                    .and_then(|n| snap.packages.iter().position(|p| p.name == n))
                    .or((!snap.packages.is_empty()).then_some(0));
                let warnings: usize = snap.repos.values().map(|r| r.errors.len()).sum();
                self.snap = Some(snap);
                self.hist = hist;
                self.table.select(idx);
                if warnings > 0 {
                    self.set_status(
                        &format!("updated · {warnings} GitHub warnings (see repo table)"),
                        true,
                    );
                } else {
                    self.set_status("updated", false);
                }
            }
            Err(e) => self.set_status(&format!("refresh failed: {e:#}"), true),
        }
    }

    pub fn set_status(&mut self, msg: &str, error: bool) {
        self.status = msg.to_string();
        self.status_is_error = error;
    }

    pub fn move_by(&mut self, delta: isize) {
        let n = self.packages().len();
        if n == 0 {
            return;
        }
        let cur = self.table.selected().unwrap_or(0) as isize;
        self.table
            .select(Some((cur + delta).clamp(0, n as isize - 1) as usize));
    }

    pub fn select_edge(&mut self, last: bool) {
        let n = self.packages().len();
        if n > 0 {
            self.table.select(Some(if last { n - 1 } else { 0 }));
        }
    }

    pub fn open(&mut self, target: OpenTarget) {
        let Some(p) = self.selected() else { return };
        let url = match target {
            OpenTarget::Aur => Some(p.aur_url.clone()),
            OpenTarget::GitHub => p
                .github_repo
                .as_ref()
                .map(|r| format!("https://github.com/{r}")),
            OpenTarget::Upstream => p.upstream_url.clone(),
        };
        let Some(url) = url else {
            self.set_status("no URL for that", true);
            return;
        };
        let spawned = Command::new("xdg-open")
            .arg(&url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match spawned {
            Ok(_) => self.set_status(&format!("opened {url}"), false),
            Err(e) => self.set_status(&format!("xdg-open failed: {e}"), true),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum OpenTarget {
    Aur,
    GitHub,
    Upstream,
}
