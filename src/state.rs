//! `installed.json` and `wishlist.json` in the state dir. Local only, never synced.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Installed {
    pub id: String,
    pub version: String,
    pub sha256: String,
    pub installed_at: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub target: String,
}

/// Local play history per title, Steam-style; never leaves the machine.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Played {
    pub last_played: String,
    pub seconds: u64,
    pub sessions: u32,
}

pub struct State {
    dir: PathBuf,
    pub installed: BTreeMap<String, Installed>,
    pub wishlist: BTreeSet<String>,
    pub played: BTreeMap<String, Played>,
}

pub fn read<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> T {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).map_err(|e| log::warn!("{}: {e}", path.display())).ok())
        .unwrap_or_default()
}

impl State {
    pub fn load(dir: &Path) -> Self {
        let installed: Vec<Installed> = read(&dir.join("installed.json"));
        State {
            dir: dir.to_path_buf(),
            installed: installed.into_iter().map(|i| (i.id.clone(), i)).collect(),
            wishlist: read(&dir.join("wishlist.json")),
            played: read(&dir.join("played.json")),
        }
    }

    /// Add a finished session to the title's play history; returns the session count.
    pub fn record_play(&mut self, id: &str, seconds: u64) -> u32 {
        let p = self.played.entry(id.to_string()).or_default();
        p.seconds += seconds;
        p.sessions += 1;
        p.last_played = now_rfc3339();
        p.sessions
    }

    pub fn save(&self) -> Result<()> {
        let list: Vec<&Installed> = self.installed.values().collect();
        write_atomic(&self.dir.join("installed.json"), &serde_json::to_string_pretty(&list)?)?;
        write_atomic(&self.dir.join("wishlist.json"), &serde_json::to_string_pretty(&self.wishlist)?)?;
        write_atomic(&self.dir.join("played.json"), &serde_json::to_string_pretty(&self.played)?)?;
        Ok(())
    }
}

/// Write via a temp file and rename, so a crash never leaves half a file.
pub fn write_atomic(path: &Path, body: &str) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// UTC timestamp in RFC 3339 form without pulling in a date crate.
pub fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // civil-from-days, Howard Hinnant's algorithm
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    #[test]
    fn timestamp_shape() {
        let t = super::now_rfc3339();
        assert_eq!(t.len(), 20);
        assert!(t.starts_with("20") && t.ends_with('Z'));
    }
}
