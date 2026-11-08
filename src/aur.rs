//! AUR RPC v5 client. Docs: https://wiki.archlinux.org/title/Aurweb_RPC_interface
//!
//! The AUR records votes and popularity but not downloads or page views.

use anyhow::{Context, Result, bail};
use serde::Deserialize;

const RPC: &str = "https://aur.archlinux.org/rpc/v5";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Package {
    pub name: String,
    pub package_base: String,
    pub version: String,
    pub description: Option<String>,
    #[serde(rename = "URL")]
    pub url: Option<String>,
    pub maintainer: Option<String>,
    pub num_votes: u32,
    pub popularity: f64,
    /// Unix time the package was flagged out-of-date, if it is.
    pub out_of_date: Option<i64>,
    pub first_submitted: i64,
    pub last_modified: i64,
}

#[derive(Deserialize)]
struct Response {
    #[serde(rename = "type")]
    kind: String,
    error: Option<String>,
    #[serde(default)]
    results: Vec<Package>,
}

fn get(agent: &ureq::Agent, url: &str) -> Result<Vec<Package>> {
    let resp: Response = agent
        .get(url)
        .call()
        .with_context(|| format!("GET {url}"))?
        .body_mut()
        .read_json()
        .context("decoding AUR response")?;
    if resp.kind == "error" {
        bail!("AUR: {}", resp.error.unwrap_or_default());
    }
    Ok(resp.results)
}

/// Every package the account maintains (search results carry all the fields we use).
pub fn by_maintainer(agent: &ureq::Agent, maintainer: &str) -> Result<Vec<Package>> {
    get(agent, &format!("{RPC}/search/{maintainer}?by=maintainer"))
}

/// Specific packages by exact name, in one request.
pub fn info(agent: &ureq::Agent, names: &[String]) -> Result<Vec<Package>> {
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let query: String = names.iter().map(|n| format!("&arg[]={n}")).collect();
    get(agent, &format!("{RPC}/info?{}", &query[1..]))
}

pub fn page_url(name: &str) -> String {
    format!("https://aur.archlinux.org/packages/{name}")
}
