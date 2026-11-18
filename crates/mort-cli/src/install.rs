//! `mort install`: put the bridge and the probe where REAPER finds them.
//!
//! Files are copied to a temporary name and renamed into place, so a
//! REAPER that has the old bridge loaded keeps its copy intact until it
//! restarts.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::client::Failure;

/// Where `mort install` reads from and writes to.
pub struct Places {
    /// The cargo target directory holding the built artefacts.
    pub from: PathBuf,
    /// REAPER's `UserPlugins` directory.
    pub user_plugins: PathBuf,
    /// The user's CLAP directory.
    pub clap: PathBuf,
}

impl Places {
    /// The defaults: artefacts beside the running `mort` binary, and the
    /// standard macOS locations.
    #[must_use]
    pub fn default_for_user() -> Places {
        let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from);
        let from = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_owned))
            .unwrap_or_else(|| PathBuf::from("."));
        Places {
            from,
            user_plugins: home.join("Library/Application Support/REAPER/UserPlugins"),
            clap: home.join("Library/Audio/Plug-Ins/CLAP"),
        }
    }
}

const BRIDGE_BUILT: &str = "libmort_bridge.dylib";
const BRIDGE_INSTALLED: &str = "reaper_mort.dylib";
const PROBE_BUILT: &str = "mort-probe.clap";

/// Copies a file or bundle into `dir` under `name`, atomically.
fn install(src: &Path, dir: &Path, name: &str) -> Result<PathBuf, Failure> {
    let fail = |what: &str, e: std::io::Error| Failure::Local(format!("{what}: {e}"));
    fs::create_dir_all(dir).map_err(|e| fail(&format!("cannot create {}", dir.display()), e))?;
    let dest = dir.join(name);
    let tmp = dir.join(format!(".{name}.installing"));
    let _ = fs::remove_dir_all(&tmp);
    let _ = fs::remove_file(&tmp);
    copy_tree(src, &tmp).map_err(|e| fail(&format!("cannot copy {}", src.display()), e))?;
    if dest.is_dir() {
        let old = dir.join(format!(".{name}.old"));
        let _ = fs::remove_dir_all(&old);
        fs::rename(&dest, &old).map_err(|e| fail("cannot move the old bundle aside", e))?;
        fs::rename(&tmp, &dest).map_err(|e| fail("cannot move the new bundle in", e))?;
        let _ = fs::remove_dir_all(&old);
    } else {
        fs::rename(&tmp, &dest)
            .map_err(|e| fail(&format!("cannot write {}", dest.display()), e))?;
    }
    Ok(dest)
}

fn copy_tree(src: &Path, dest: &Path) -> std::io::Result<()> {
    if src.is_dir() {
        fs::create_dir_all(dest)?;
        for e in fs::read_dir(src)? {
            let e = e?;
            copy_tree(&e.path(), &dest.join(e.file_name()))?;
        }
        Ok(())
    } else {
        fs::copy(src, dest).map(|_| ())
    }
}

/// Whether a REAPER process is running.
#[must_use]
pub fn reaper_running() -> bool {
    Command::new("pgrep")
        .args(["-x", "REAPER"])
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Installs what has been built, and returns lines to print.
///
/// # Errors
///
/// [`Failure::Local`] if the bridge has not been built or cannot be
/// copied.
pub fn run(places: &Places, reaper_running: bool) -> Result<Vec<String>, Failure> {
    let bridge = places.from.join(BRIDGE_BUILT);
    if !bridge.is_file() {
        return Err(Failure::Local(format!(
            "{} is missing; build it with `cargo build -p mort-bridge` (add --release for a \
             release install) and run the `mort` from the same target directory, or pass --from",
            bridge.display()
        )));
    }
    let mut out = Vec::new();
    let dest = install(&bridge, &places.user_plugins, BRIDGE_INSTALLED)?;
    out.push(format!("bridge  {}", dest.display()));
    let probe = places.from.join(PROBE_BUILT);
    if probe.exists() {
        let dest = install(&probe, &places.clap, PROBE_BUILT)?;
        out.push(format!("probe   {}", dest.display()));
    } else {
        out.push(format!(
            "probe   not built ({} is missing); skipped",
            probe.display()
        ));
    }
    if reaper_running {
        out.push("REAPER is running: restart it to load what was installed".into());
    }
    Ok(out)
}
