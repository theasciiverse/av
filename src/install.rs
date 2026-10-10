//! Download, checksum, extract, link. Runs on a worker thread and reports
//! progress over a channel. Every archive is checked against the catalog's
//! sha256 before anything is extracted.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use crate::catalog::{self, Title};
use crate::state::Installed;

pub enum Progress {
    Bytes { id: String, done: u64, total: u64 },
    Status { id: String, msg: String },
    Done { id: String, installed: Installed },
    Failed { id: String, error: String },
}

pub struct Job {
    pub title: Title,
    pub catalog_url: String,
    pub cache: PathBuf,
    pub dest: PathBuf,
    pub bin: Option<PathBuf>,
}

pub fn spawn(job: Job, tx: Sender<Progress>) {
    std::thread::spawn(move || {
        let id = job.title.id.clone();
        let msg = match run(&job, &tx) {
            Ok(installed) => Progress::Done { id, installed },
            Err(e) => {
                log::error!("install {id} failed: {e:#}");
                Progress::Failed { id, error: format!("{e:#}") }
            }
        };
        let _ = tx.send(msg);
    });
}

fn run(job: &Job, tx: &Sender<Progress>) -> Result<Installed> {
    let t = &job.title;
    let (target, plat) = catalog::pick_platform(t).context("no build for this platform")?;
    let url = catalog::resolve(&job.catalog_url, &plat.url);
    let name = url.rsplit('/').next().unwrap_or("archive").to_string();
    let downloads = job.cache.join("downloads");
    std::fs::create_dir_all(&downloads)?;
    let archive = downloads.join(&name);
    log::info!("install {}: {} from {url}", t.id, target);

    // download while hashing
    let status = |msg: &str| {
        let _ = tx.send(Progress::Status { id: t.id.clone(), msg: msg.into() });
    };
    status("downloading");
    let mut resp = ureq::get(&url).call().with_context(|| format!("download {url}"))?;
    let total = resp.body().content_length().unwrap_or(plat.size);
    let mut reader = resp.body_mut().as_reader();
    let mut file = std::fs::File::create(&archive)?;
    let mut hasher = Sha256::new();
    let (mut buf, mut done) = (vec![0u8; 64 * 1024], 0u64);
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])?;
        done += n as u64;
        let _ = tx.send(Progress::Bytes { id: t.id.clone(), done, total });
    }
    file.sync_all()?;
    let got = hex(&hasher.finalize());
    if !got.eq_ignore_ascii_case(&plat.sha256) {
        let _ = std::fs::remove_file(&archive);
        bail!("checksum mismatch: the catalog says {}, the download is {got}. Not installed.", plat.sha256);
    }
    status("checksum ok, extracting");

    // extract into a fresh sibling dir, then swap it in, so a failed update
    // never leaves a half-written install
    let parent = job.dest.parent().context("bad install path")?;
    std::fs::create_dir_all(parent)?;
    let staging = parent.join(format!(".{}.new", t.id));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;
    if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        let gz = flate2::read::GzDecoder::new(std::fs::File::open(&archive)?);
        tar::Archive::new(gz).unpack(&staging).context("extract tar.gz")?;
    } else {
        bail!("unsupported archive {name} (zip support is not built yet)");
    }
    let exe = find_exec(&staging, &t.exec_name()).with_context(|| format!("{} not found in the archive", t.exec_name()))?;
    set_executable(&exe)?;
    let _ = std::fs::remove_dir_all(&job.dest);
    std::fs::rename(&staging, &job.dest)?;
    let exe = job.dest.join(exe.strip_prefix(&staging)?);

    if let Some(bin) = &job.bin {
        link(bin, &t.id, &exe)?;
    }
    let _ = std::fs::remove_file(&archive);
    Ok(Installed {
        id: t.id.clone(),
        version: t.version.clone(),
        sha256: got,
        installed_at: crate::state::now_rfc3339(),
        kind: t.kind.clone(),
        target: target.into(),
    })
}

/// The binary may sit at the top of the archive or inside one directory.
pub fn find_exec(dir: &Path, name: &str) -> Option<PathBuf> {
    let top = dir.join(name);
    if top.is_file() {
        return Some(top);
    }
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path().join(name)).find(|p| p.is_file())
}

/// Set the executable bit explicitly instead of trusting the archive.
fn set_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut p = std::fs::metadata(path)?.permissions();
        p.set_mode(p.mode() | 0o755);
        std::fs::set_permissions(path, p)?;
    }
    let _ = path;
    Ok(())
}

fn link(bin: &Path, id: &str, exe: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        std::fs::create_dir_all(bin)?;
        let l = bin.join(id);
        if l.symlink_metadata().is_ok() {
            // only replace links av made; never clobber a user's own file
            if std::fs::read_link(&l).is_err() {
                log::warn!("{} exists and is not a symlink; leaving it", l.display());
                return Ok(());
            }
            std::fs::remove_file(&l)?;
        }
        std::os::unix::fs::symlink(exe, &l)?;
    }
    let _ = (bin, id, exe);
    Ok(())
}

pub fn uninstall(dest: &Path, bin: Option<&Path>, id: &str) -> Result<()> {
    if let Some(bin) = bin {
        let l = bin.join(id);
        if std::fs::read_link(&l).map(|t| t.starts_with(dest)).unwrap_or(false) {
            std::fs::remove_file(&l)?;
        }
    }
    if dest.exists() {
        std::fs::remove_dir_all(dest)?;
    }
    Ok(())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
