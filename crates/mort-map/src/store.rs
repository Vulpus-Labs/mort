//! The sidecar (ADR 0006 §6): captures saved next to the project file.
//!
//! `<dir>/<stem>.mort/captures/` holds `index.json`, with the format
//! version, the next capture id, every kept capture's metadata and each
//! track's covers, and `<id>.frames` files with the frames the captures
//! took. A capture's frames are in the file named by its `file`, or by its
//! own id: the stretches of one playback share one file (ADR 0012 §5).
//! Frames never change, so a file is rewritten only while captures are
//! still being added to it:
//!
//! ```text
//! "MORTFRM3"
//! u32 capture count
//! per capture: u64 id, u32 track count,
//!   per track: u128 track GUID, u32 frame count,
//!              per frame: f64 start, 220-byte frame (Frame::to_le_bytes)
//! ```
//!
//! Format 2 files (`MORTFRM2`) hold one capture, named by the file, with no
//! capture header; they still load.
//!
//! All numbers are little-endian.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use mort_features::{FRAME_BYTES, Frame};
use mort_proto::{Guid, TrackId};

use crate::{Capture, Cover, MixMap, TimedFrame};

const MAGIC: &[u8; 8] = b"MORTFRM3";
const MAGIC_V2: &[u8; 8] = b"MORTFRM2";
const INDEX: &str = "index.json";
const VERSION: u32 = 3;

#[derive(Serialize, Deserialize)]
struct Index {
    version: u32,
    next: u64,
    captures: Vec<Capture>,
    covers: BTreeMap<TrackId, Vec<Cover>>,
}

/// The sidecar directory for a project file.
#[must_use]
pub fn dir_for(project: &Path) -> PathBuf {
    let stem = project
        .file_stem()
        .map_or_else(|| "project".into(), |s| s.to_string_lossy().into_owned());
    project
        .with_file_name(format!("{stem}.mort"))
        .join("captures")
}

fn bad(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

/// Writes the map to `dir`: the frames of captures not yet written (all
/// of them if `all`), the index, and removes the files of captures the map
/// no longer keeps. Marks the map saved.
///
/// # Errors
///
/// Any I/O error.
pub fn save(dir: &Path, map: &mut MixMap, all: bool) -> io::Result<()> {
    if !all && !map.changed() {
        return Ok(());
    }
    fs::create_dir_all(dir)?;
    let file_of = |c: &Capture| c.file.unwrap_or(c.id);
    let files: std::collections::BTreeSet<u64> = map.all_captures().map(file_of).collect();
    for f in &files {
        let members: Vec<u64> = map
            .all_captures()
            .filter(|c| file_of(c) == *f)
            .map(|c| c.id)
            .collect();
        if all || members.iter().any(|id| map.unsaved().contains(id)) {
            write_atomic(
                &dir.join(format!("{f}.frames")),
                &frames_file(map, &members),
            )?;
        }
    }
    let ids: Vec<u64> = files.into_iter().collect();
    let index = Index {
        version: VERSION,
        next: map.next_id(),
        captures: map.all_captures().cloned().collect(),
        covers: map.all_covers().clone(),
    };
    let json = serde_json::to_vec_pretty(&index).map_err(|e| bad(e.to_string()))?;
    write_atomic(&dir.join(INDEX), &json)?;
    for entry in fs::read_dir(dir)? {
        let p = entry?.path();
        let gone = p.extension().is_some_and(|e| e == "frames")
            && p.file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse::<u64>().ok())
                .is_none_or(|id| !ids.contains(&id));
        if gone {
            fs::remove_file(&p)?;
        }
    }
    map.mark_saved();
    Ok(())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

fn frames_file(map: &MixMap, ids: &[u64]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(MAGIC);
    out.extend(u32::try_from(ids.len()).unwrap_or(u32::MAX).to_le_bytes());
    for &id in ids {
        let tracks: Vec<TrackId> = map.measured(id).collect();
        out.extend(id.to_le_bytes());
        out.extend(
            u32::try_from(tracks.len())
                .unwrap_or(u32::MAX)
                .to_le_bytes(),
        );
        for t in tracks {
            let fs = map.capture_frames(id, t);
            out.extend(t.0.0.to_le_bytes());
            out.extend(u32::try_from(fs.len()).unwrap_or(u32::MAX).to_le_bytes());
            for f in fs {
                out.extend(f.start.to_le_bytes());
                out.extend(f.frame.to_le_bytes());
            }
        }
    }
    out
}

struct Cursor<'a> {
    b: &'a [u8],
    at: usize,
}

impl Cursor<'_> {
    fn take<const N: usize>(&mut self) -> io::Result<[u8; N]> {
        let s = self
            .b
            .get(self.at..self.at + N)
            .ok_or_else(|| bad("a frames file ends early"))?;
        self.at += N;
        Ok(s.try_into().expect("length checked"))
    }

    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.take()?))
    }

    fn f64(&mut self) -> io::Result<f64> {
        Ok(f64::from_le_bytes(self.take()?))
    }
}

type Frames = BTreeMap<TrackId, Vec<TimedFrame>>;

fn read_tracks(c: &mut Cursor<'_>) -> io::Result<Frames> {
    let mut out = BTreeMap::new();
    for _ in 0..c.u32()? {
        let track = TrackId(Guid(u128::from_le_bytes(c.take()?)));
        let n = c.u32()? as usize;
        let mut frames = Vec::with_capacity(n.min(1 << 20));
        for _ in 0..n {
            let start = c.f64()?;
            let frame = Frame::from_le_bytes(&c.take::<FRAME_BYTES>()?);
            frames.push(TimedFrame { start, frame });
        }
        out.insert(track, frames);
    }
    Ok(out)
}

/// The captures in frames file `file`, by id.
fn read_frames(bytes: &[u8], file: u64) -> io::Result<BTreeMap<u64, Frames>> {
    let mut c = Cursor { b: bytes, at: 0 };
    let magic = c.take::<8>()?;
    let mut out = BTreeMap::new();
    if &magic == MAGIC_V2 {
        out.insert(file, read_tracks(&mut c)?);
    } else if &magic == MAGIC {
        for _ in 0..c.u32()? {
            let id = u64::from_le_bytes(c.take()?);
            out.insert(id, read_tracks(&mut c)?);
        }
    } else {
        return Err(bad(format!("{file}.frames is not a mort frames file")));
    }
    Ok(out)
}

/// Reads the map saved in `dir`, or `None` if there is none.
///
/// # Errors
///
/// An I/O error, or a file that is not what [`save`] writes.
pub fn load(dir: &Path) -> io::Result<Option<MixMap>> {
    let index = match fs::read(dir.join(INDEX)) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let index: Index = serde_json::from_slice(&index).map_err(|e| bad(e.to_string()))?;
    if !(2..=VERSION).contains(&index.version) {
        return Err(bad(format!(
            "captures saved in format {}; this mort reads {VERSION}: delete the directory and measure again",
            index.version
        )));
    }
    let mut frames = BTreeMap::new();
    let files: std::collections::BTreeSet<u64> = index
        .captures
        .iter()
        .map(|c| c.file.unwrap_or(c.id))
        .collect();
    for f in files {
        match fs::read(dir.join(format!("{f}.frames"))) {
            Ok(b) => {
                frames.extend(read_frames(&b, f)?);
            }
            // A capture without frames is skipped, not fatal.
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(Some(MixMap::from_parts(
        index.next,
        index.captures,
        frames,
        index.covers,
    )))
}
