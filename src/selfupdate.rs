//! av's own updates. The catalog names the latest av in its top-level `av`
//! entry; when that is newer than this one, av says so, and how to update,
//! which depends on how this av was installed.

use crate::catalog::AvRelease;

/// The catalog's av, when it is newer than this one.
pub fn newer(latest: Option<&AvRelease>) -> Option<&AvRelease> {
    latest.filter(|r| version(&r.version) > version(env!("CARGO_PKG_VERSION")))
}

/// "0.9.10" as [0, 9, 10], so it sorts after 0.9.9; a pre-release or build
/// suffix is left out.
fn version(v: &str) -> Vec<u64> {
    v.split(['-', '+']).next().unwrap_or("").split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

/// The command that updates this av. The install line updates both the
/// .deb and a per-user install (it asks which); cargo builds its own.
pub fn how() -> String {
    if cfg!(windows) {
        return "irm https://asciiverse.sh/install.ps1 | iex".into();
    }
    let exe = std::env::current_exe().ok().and_then(|p| p.canonicalize().ok()).unwrap_or_default();
    if exe.components().any(|c| c.as_os_str() == ".cargo") {
        return "cargo install --locked --git https://github.com/theasciiverse/av".into();
    }
    // Ubuntu's desktop has wget but not curl; macOS has curl
    if on_path("curl") || !on_path("wget") {
        "curl -fsSL https://asciiverse.sh/install.sh | sh".into()
    } else {
        "wget -qO- https://asciiverse.sh/install.sh | sh".into()
    }
}

/// One line for the shell and the status line: "av 0.9.8 is out (you have
/// 0.9.7); to update: <command>".
pub fn notice(r: &AvRelease) -> String {
    format!("av {} is out (you have {}); to update: {}", r.version, env!("CARGO_PKG_VERSION"), how())
}

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(program).is_file()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number() {
        assert!(version("0.9.10") > version("0.9.9"));
        assert!(version("1.0.0") > version("0.99.0"));
        assert_eq!(version("1.2.3-rc1"), version("1.2.3"));
        let release = |v: &str| AvRelease { version: v.into(), ..Default::default() };
        assert!(newer(Some(&release("999.0.0"))).is_some());
        assert!(newer(Some(&release(env!("CARGO_PKG_VERSION")))).is_none());
        assert!(newer(Some(&release("0.0.1"))).is_none());
        assert!(newer(None).is_none());
    }
}
