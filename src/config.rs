//! `~/.config/aurwatch/config.toml` — who to watch and where their sources live.

use std::{collections::BTreeMap, fs, path::PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// AUR account whose packages are discovered automatically.
    pub maintainer: String,
    /// Extra packages to watch that this maintainer does not own (co-maintained, etc.).
    pub extra_packages: Vec<String>,
    /// Package name -> GitHub `owner/repo`, for when it can't be inferred from the AUR URL.
    pub repos: BTreeMap<String, String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            maintainer: "darkstardevx".into(),
            extra_packages: Vec::new(),
            repos: BTreeMap::new(),
        }
    }
}

const TEMPLATE: &str = r#"# aurwatch configuration

# AUR account whose packages are discovered automatically.
maintainer = "darkstardevx"

# Packages to watch that this account doesn't maintain.
extra_packages = []

# Package -> GitHub owner/repo, only needed when aurwatch can't infer it from
# the package URL (github.com/<owner>/<repo> or <owner>.github.io/<repo>).
[repos]
# dwcode = "cybercoretech/dwcode"
"#;

pub fn path() -> Result<PathBuf> {
    Ok(dirs::config_dir()
        .context("no XDG config directory")?
        .join("aurwatch/config.toml"))
}

/// Load the config, writing the commented template on first run.
pub fn load() -> Result<Config> {
    let path = path()?;
    if !path.exists() {
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(&path, TEMPLATE).with_context(|| format!("writing {}", path.display()))?;
    }
    let raw = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))
}
