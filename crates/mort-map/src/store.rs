//! The sidecar (ADR 0006 §6): captures saved next to the project file.
//!
//! `<dir>/<stem>.mort/captures/` holds `index.json`, with the next capture
//! id and every capture's metadata, and one `<id>.frames` file per
//! capture with its segments:
//!
//! ```text
//! "MORTFRM1"
//! u32 segment count
//! per segment: u128 track GUID, f64 start, f64 end, u32 frame count,
//!              per frame: f64 start, 220-byte frame (Frame::to_le_bytes)
//! ```
//!
//! All numbers are little-endian.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use mort_features::{FRAME_BYTES, Frame};
use mort_proto::{Guid, TrackId};

use crate::{Capture, MixMap, Segment, Span, TimedFrame};

const MAGIC: &[u8; 8] = b"MORTFRM1";
const INDEX: &str = "index.json";
const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct Index {
    version: u32,
    next: u64,
    captures: Vec<Capture>,
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

/// Writes the map to `dir`: the index, the frames of captures touched
/// since the last save (all of them if `all`), and removes the files of
/// captures that are gone. Marks the map saved.
///
/// # Errors
///
/// Any I/O error.
pub fn save(dir: &Path, map: &mut MixMap, all: bool) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let ids: Vec<u64> = map.captures().map(|c| c.id).collect();
    for id in &ids {
        if all || map.touched().contains(id) {
            write_atomic(&dir.join(format!("{id}.frames")), &frames_file(map, *id))?;
        }
    }
    let index = Index {
        version: VERSION,
        next: map.next_id(),
        captures: map.captures().cloned().collect(),
    };
    let json = serde_json::to_vec_pretty(&index).map_err(|e| bad(e.to_string()))?;
    write_atomic(&dir.join(INDEX), &json)?;
    for entry in fs::read_dir(dir)? {
        let p = entry?.path();
        let stale = p.extension().is_some_and(|e| e == "frames")
            && p.file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse::<u64>().ok())
                .is_none_or(|id| !ids.contains(&id));
        if stale {
            fs::remove_file(&p)?;
        }
    }
    map.untouch();
    Ok(())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

fn frames_file(map: &MixMap, id: u64) -> Vec<u8> {
    let segs: Vec<(TrackId, &Segment)> = map
        .tracks()
        .flat_map(|t| {
            map.segments(t)
                .iter()
                .filter(|s| s.capture == id)
                .map(move |s| (t, s))
        })
        .collect();
    let n: usize = segs.iter().map(|(_, s)| s.frames.len()).sum();
    let mut out = Vec::with_capacity(12 + segs.len() * 36 + n * (8 + FRAME_BYTES));
    out.extend(MAGIC);
    out.extend(u32::try_from(segs.len()).unwrap_or(u32::MAX).to_le_bytes());
    for (t, s) in segs {
        out.extend(t.0.0.to_le_bytes());
        out.extend(s.span.start.to_le_bytes());
        out.extend(s.span.end.to_le_bytes());
        out.extend(
            u32::try_from(s.frames.len())
                .unwrap_or(u32::MAX)
                .to_le_bytes(),
        );
        for f in &s.frames {
            out.extend(f.start.to_le_bytes());
            out.extend(f.frame.to_le_bytes());
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

fn read_frames(
    bytes: &[u8],
    id: u64,
    tracks: &mut BTreeMap<TrackId, Vec<Segment>>,
) -> io::Result<()> {
    let mut c = Cursor { b: bytes, at: 0 };
    if &c.take::<8>()? != MAGIC {
        return Err(bad(format!("{id}.frames is not a mort frames file")));
    }
    for _ in 0..c.u32()? {
        let track = TrackId(Guid(u128::from_le_bytes(c.take()?)));
        let span = Span::new(c.f64()?, c.f64()?);
        let n = c.u32()? as usize;
        let mut frames = Vec::with_capacity(n.min(1 << 20));
        for _ in 0..n {
            let start = c.f64()?;
            let frame = Frame::from_le_bytes(&c.take::<FRAME_BYTES>()?);
            frames.push(TimedFrame { start, frame });
        }
        tracks.entry(track).or_default().push(Segment {
            capture: id,
            span,
            frames,
        });
    }
    Ok(())
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
    if index.version != VERSION {
        return Err(bad(format!(
            "captures saved in format {}; this mort reads {VERSION}",
            index.version
        )));
    }
    let mut tracks = BTreeMap::new();
    let mut captures = Vec::new();
    for c in index.captures {
        match fs::read(dir.join(format!("{}.frames", c.id))) {
            Ok(b) => {
                read_frames(&b, c.id, &mut tracks)?;
                captures.push(c);
            }
            // A capture without frames is skipped, not fatal.
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(Some(MixMap::from_parts(index.next, captures, tracks)))
}
