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
    let mut resp = catalog::http().get(&url).call().with_context(|| format!("download {url}"))?;
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
    use super::{Job, carry_over, extract, find_exec, run, uninstall};
    use crate::catalog::{self, Title};
    use std::collections::BTreeMap;
    use std::fs;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::path::{Path, PathBuf};
    use std::sync::mpsc::channel;

    /// A tiny HTTP server on 127.0.0.1 that serves `files` by path, for as
    /// long as the test runs. Returns its base URL.
    fn serve(files: BTreeMap<String, Vec<u8>>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut line = String::new();
                if BufReader::new(&stream).read_line(&mut line).is_err() {
                    continue;
                }
                let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                let head = |status: &str, len: usize| format!("HTTP/1.1 {status}\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n");
                match files.get(&path) {
                    Some(body) => {
                        let _ = stream.write_all(head("200 OK", body.len()).as_bytes());
                        let _ = stream.write_all(body);
                    }
                    None => {
                        let _ = stream.write_all(head("404 Not Found", 0).as_bytes());
                    }
                }
            }
        });
        base
    }

    /// A release archive as the catalog expects: the binary one folder down,
    /// executable, beside a data file.
    fn tar_gz(binary: &str, data: &str) -> Vec<u8> {
        let mut out = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast()));
        for (path, body, mode) in [("game/game", binary, 0o755), ("game/data.txt", data, 0o644)] {
            let mut h = tar::Header::new_gnu();
            h.set_size(body.len() as u64);
            h.set_mode(mode);
            h.set_cksum();
            out.append_data(&mut h, path, body.as_bytes()).unwrap();
        }
        out.into_inner().unwrap().finish().unwrap()
    }

    fn sha(b: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        super::hex(&Sha256::digest(b))
    }

    /// A catalog title for this machine, served from `url`, with `sha256`.
    fn title(version: &str, url: &str, sha256: &str) -> Title {
        let target = catalog::host_targets()[0];
        let platform = format!(r#"{{"url": "{url}", "sha256": "{sha256}", "size": 0}}"#);
        let json = format!(r#"{{"id": "game", "kind": "game", "version": "{version}", "platforms": {{"{target}": {platform}}}}}"#);
        serde_json::from_str(&json).unwrap()
    }

    struct Home {
        root: PathBuf,
    }

    impl Home {
        fn new(name: &str) -> Home {
            let root = std::env::temp_dir().join(format!("av-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            Home { root }
        }
        fn dest(&self) -> PathBuf {
            self.root.join("games/game")
        }
        fn bin(&self) -> PathBuf {
            self.root.join("bin")
        }
        fn job(&self, title: Title, catalog_url: &str) -> Job {
            Job { title, catalog_url: catalog_url.into(), cache: self.root.join("cache"), dest: self.dest(), bin: Some(self.bin()), cancel: None }
        }
    }

    impl Drop for Home {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn read(p: &Path) -> String {
        fs::read_to_string(p).unwrap()
    }

    #[test]
    fn install_update_and_uninstall_from_a_catalog() {
        let (v1, v2) = (tar_gz("binary 1", "data 1"), tar_gz("binary 2", "data 2"));
        let (sha1, sha2) = (sha(&v1), sha(&v2));
        let base = serve(BTreeMap::from([("/r/game-1.tar.gz".to_string(), v1), ("/r/game-2.tar.gz".to_string(), v2)]));
        let catalog_url = format!("{base}/data/catalog.json");
        let home = Home::new("install");
        let (tx, rx) = channel();

        // install: the archive's url is relative to the catalog's host, as on the website
        let done = run(&home.job(title("1.0", "/r/game-1.tar.gz", &sha1), &catalog_url), &tx).unwrap();
        assert_eq!((done.version.as_str(), done.sha256.as_str()), ("1.0", sha1.as_str()));
        let exe = home.dest().join("game/game");
        assert_eq!(read(&exe), "binary 1");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_ne!(fs::metadata(&exe).unwrap().permissions().mode() & 0o111, 0, "the binary is executable");
            assert_eq!(fs::read_link(home.bin().join("game")).unwrap(), exe);
        }
        assert!(rx.try_iter().any(|p| matches!(p, super::Progress::Status { msg, .. } if msg.starts_with("checksum ok"))));
        assert!(!home.root.join("cache/downloads/game-1.tar.gz").exists(), "the archive is removed after installing");

        // the title writes a save next to itself; an update keeps it
        fs::write(home.dest().join("game/save.dat"), "my save").unwrap();
        let done = run(&home.job(title("2.0", &format!("{base}/r/game-2.tar.gz"), &sha2), &catalog_url), &tx).unwrap();
        assert_eq!(done.version, "2.0");
        assert_eq!(read(&exe), "binary 2");
        assert_eq!(read(&home.dest().join("game/data.txt")), "data 2");
        assert_eq!(read(&home.dest().join("game/save.dat")), "my save");

        uninstall(&home.dest(), Some(&home.bin()), "game").unwrap();
        assert!(!home.dest().exists());
        assert!(home.bin().join("game").symlink_metadata().is_err(), "the link is gone");
    }

    #[test]
    fn a_download_that_does_not_match_its_checksum_is_not_installed() {
        let archive = tar_gz("binary", "data");
        let base = serve(BTreeMap::from([("/r/game.tar.gz".to_string(), archive)]));
        let home = Home::new("checksum");
        let (tx, _rx) = channel();
        let wrong = "0".repeat(64);
        let err = run(&home.job(title("1.0", "/r/game.tar.gz", &wrong), &format!("{base}/catalog.json")), &tx).unwrap_err();
        assert!(err.to_string().starts_with("checksum mismatch"), "{err:#}");
        assert!(!home.dest().exists(), "nothing is extracted");
        assert!(!home.root.join("cache/downloads/game.tar.gz").exists(), "the bad download is deleted");
        assert!(home.bin().join("game").symlink_metadata().is_err());
    }

    #[test]
    fn a_failed_update_leaves_the_old_install_alone() {
        let good = tar_gz("binary 1", "data 1");
        let sha1 = sha(&good);
        let base = serve(BTreeMap::from([("/r/game-1.tar.gz".to_string(), good)]));
        let catalog_url = format!("{base}/catalog.json");
        let home = Home::new("failed-update");
        let (tx, _rx) = channel();
        run(&home.job(title("1.0", "/r/game-1.tar.gz", &sha1), &catalog_url), &tx).unwrap();
        // version 2's archive is missing from the server
        assert!(run(&home.job(title("2.0", "/r/game-2.tar.gz", &sha1), &catalog_url), &tx).is_err());
        assert_eq!(read(&home.dest().join("game/game")), "binary 1");
    }

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
