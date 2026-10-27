//! Download, checksum, extract, link. Runs on a worker thread and reports
//! progress over a channel. Every archive is checked against the catalog's
//! sha256 before anything is extracted.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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
    /// set from outside to stop the download; checked per chunk
    pub cancel: Option<Arc<AtomicBool>>,
}

pub fn spawn(job: Job, tx: Sender<Progress>) {
    std::thread::spawn(move || {
        let id = job.title.id.clone();
        let msg = match run(&job, &tx) {
            Ok(installed) => Progress::Done { id, installed },
            Err(e) if e.to_string() == "cancelled" => {
                log::info!("install {id} cancelled");
                Progress::Failed { id, error: "cancelled".into() }
            }
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
        if job.cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed)) {
            drop(file);
            let _ = std::fs::remove_file(&archive);
            bail!("cancelled");
        }
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
    extract(&archive, &name, &staging)?;
    let exe = find_exec(&staging, &t.exec_name()).with_context(|| format!("{} not found in the archive", t.exec_name()))?;
    set_executable(&exe)?;
    // an update keeps what the title wrote next to itself (saves, scores,
    // config): anything in the old install that the new archive lacks
    if job.dest.is_dir() {
        carry_over(&job.dest, &staging).context("keep files from the previous install")?;
    }
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

/// Unpack by the archive's name: `.tar.gz` on Linux and macOS, `.zip` on
/// Windows. Both refuse entries that would land outside `dest`.
fn extract(archive: &Path, name: &str, dest: &Path) -> Result<()> {
    let file = std::fs::File::open(archive)?;
    if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        tar::Archive::new(flate2::read::GzDecoder::new(file)).unpack(dest).context("extract tar.gz")?;
    } else if name.ends_with(".zip") {
        zip::ZipArchive::new(file).and_then(|mut z| z.extract(dest)).context("extract zip")?;
    } else {
        bail!("unsupported archive {name}: av unpacks .tar.gz and .zip");
    }
    Ok(())
}

/// Move every file or folder under `old` that has no counterpart under `new`
/// into `new`, keeping its relative path. Folders present in both are merged
/// the same way; files present in both keep the new archive's copy. Both sit
/// in the same parent folder, so these are renames, not copies. A file an
/// older release shipped and a newer one dropped is carried over too: a stray
/// file is harmless, a lost save is not.
fn carry_over(old: &Path, new: &Path) -> Result<()> {
    for entry in std::fs::read_dir(old)? {
        let entry = entry?;
        let (from, to) = (entry.path(), new.join(entry.file_name()));
        let to_meta = std::fs::symlink_metadata(&to);
        if entry.file_type()?.is_dir() && to_meta.as_ref().is_ok_and(|m| m.is_dir()) {
            carry_over(&from, &to)?;
        } else if to_meta.is_err() {
            std::fs::rename(&from, &to)?;
        }
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::{carry_over, extract, find_exec};
    use std::fs;
    use std::io::Write;

    #[test]
    fn zip_extracts_with_the_binary_one_folder_down() {
        let root = std::env::temp_dir().join(format!("av-zip-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let archive = root.join("game-x86_64-pc-windows-msvc.zip");
        let mut z = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        z.add_directory("game/data/", opts).unwrap();
        z.start_file("game/game.exe", opts).unwrap();
        z.write_all(b"binary").unwrap();
        z.start_file("game/data/levels.txt", opts).unwrap();
        z.write_all(b"level 1").unwrap();
        z.finish().unwrap();

        let dest = root.join("out");
        extract(&archive, "game-x86_64-pc-windows-msvc.zip", &dest).unwrap();
        assert_eq!(find_exec(&dest, "game.exe"), Some(dest.join("game/game.exe")));
        assert_eq!(fs::read_to_string(dest.join("game/data/levels.txt")).unwrap(), "level 1");
        assert!(extract(&archive, "game.7z", &dest).is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn update_keeps_files_the_archive_does_not_have() {
        let root = std::env::temp_dir().join(format!("av-carry-over-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (old, new) = (root.join("old"), root.join("new"));
        fs::create_dir_all(old.join("nki")).unwrap();
        fs::create_dir_all(old.join("saves")).unwrap();
        fs::write(old.join("game"), "old binary").unwrap();
        fs::write(old.join("empire.sav"), "a save").unwrap();
        fs::write(old.join("nki/vanilla.nki"), "old data").unwrap();
        fs::write(old.join("nki/custom.nki"), "player's own").unwrap();
        fs::write(old.join("saves/slot1"), "slot").unwrap();
        fs::create_dir_all(new.join("nki")).unwrap();
        fs::write(new.join("game"), "new binary").unwrap();
        fs::write(new.join("nki/vanilla.nki"), "new data").unwrap();

        carry_over(&old, &new).unwrap();

        let read = |p: &str| fs::read_to_string(new.join(p)).unwrap();
        assert_eq!(read("game"), "new binary");
        assert_eq!(read("nki/vanilla.nki"), "new data");
        assert_eq!(read("empire.sav"), "a save");
        assert_eq!(read("nki/custom.nki"), "player's own");
        assert_eq!(read("saves/slot1"), "slot");
        let _ = fs::remove_dir_all(&root);
    }
}
