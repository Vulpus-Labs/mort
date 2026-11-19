//! Reading notes and control changes from a Standard MIDI File.
//!
//! Only what `mort midi --file` needs: formats 0 and 1, note on/off with
//! running status, control changes, and the file's ticks per quarter note.
//! Times come out in quarter notes from the start of the file; tempo events
//! are ignored, because the project's tempo map places the notes.

/// A note, in quarter notes from the file's start.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SmfNote {
    /// MIDI note number.
    pub pitch: u8,
    /// Start.
    pub start: f64,
    /// End.
    pub end: f64,
    /// Velocity, 1–127.
    pub velocity: u8,
    /// Channel, 0–15.
    pub channel: u8,
}

/// A control change, in quarter notes from the file's start.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SmfCc {
    /// When.
    pub time: f64,
    /// Controller number.
    pub controller: u8,
    /// Value.
    pub value: u8,
    /// Channel, 0–15.
    pub channel: u8,
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn byte(&mut self) -> Result<u8, String> {
        let v = *self
            .b
            .get(self.at)
            .ok_or("the file ends in the middle of an event")?;
        self.at += 1;
        Ok(v)
    }

    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let end = self
            .at
            .checked_add(n)
            .filter(|e| *e <= self.b.len())
            .ok_or("the file ends early")?;
        let s = &self.b[self.at..end];
        self.at = end;
        Ok(s)
    }

    fn u32(&mut self) -> Result<u32, String> {
        let s = self.take(4)?;
        Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }

    fn u16(&mut self) -> Result<u16, String> {
        let s = self.take(2)?;
        Ok(u16::from_be_bytes([s[0], s[1]]))
    }

    fn var(&mut self) -> Result<u32, String> {
        let mut v = 0u32;
        for _ in 0..4 {
            let b = self.byte()?;
            v = (v << 7) | u32::from(b & 0x7F);
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err("a variable-length number is too long".into())
    }
}

/// Reads notes and control changes from SMF bytes.
///
/// # Errors
///
/// If the bytes are not a Standard MIDI File this reader understands, with
/// the reason.
pub fn read(bytes: &[u8]) -> Result<(Vec<SmfNote>, Vec<SmfCc>), String> {
    let mut r = Reader { b: bytes, at: 0 };
    if r.take(4).map_err(|_| "not a MIDI file")? != b"MThd" {
        return Err("not a MIDI file (no MThd header)".into());
    }
    let hlen = r.u32()? as usize;
    let format = r.u16()?;
    let ntracks = r.u16()?;
    let division = r.u16()?;
    r.take(hlen.saturating_sub(6))?;
    if format > 1 {
        return Err(format!(
            "MIDI file format {format} is not supported; save it as format 0 or 1"
        ));
    }
    if division & 0x8000 != 0 {
        return Err(
            "SMPTE-timed MIDI files are not supported; save it with ticks per quarter note".into(),
        );
    }
    let tpq = f64::from(division.max(1));
    let mut notes = Vec::new();
    let mut ccs = Vec::new();
    for _ in 0..ntracks {
        if r.take(4)? != b"MTrk" {
            return Err("a track chunk is missing its MTrk header".into());
        }
        let len = r.u32()? as usize;
        let body = r.take(len)?;
        read_track(body, tpq, &mut notes, &mut ccs)?;
    }
    notes.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.pitch.cmp(&b.pitch)));
    ccs.sort_by(|a, b| a.time.total_cmp(&b.time));
    Ok((notes, ccs))
}

/// Reads one `MTrk` body into `notes` and `ccs`.
fn read_track(
    body: &[u8],
    tpq: f64,
    notes: &mut Vec<SmfNote>,
    ccs: &mut Vec<SmfCc>,
) -> Result<(), String> {
    let mut t = Reader { b: body, at: 0 };
    let mut tick = 0u64;
    let mut status = 0u8;
    // (channel, pitch) -> (start tick, velocity)
    let mut open: std::collections::HashMap<(u8, u8), Vec<(u64, u8)>> =
        std::collections::HashMap::new();
    while t.at < body.len() {
        tick += u64::from(t.var()?);
        let mut b = t.byte()?;
        if b == 0xFF {
            let kind = t.byte()?;
            let n = t.var()? as usize;
            t.take(n)?;
            if kind == 0x2F {
                break;
            }
            continue;
        }
        if b == 0xF0 || b == 0xF7 {
            let n = t.var()? as usize;
            t.take(n)?;
            continue;
        }
        let first = if b & 0x80 == 0 {
            if status == 0 {
                return Err("running status with no status before it".into());
            }
            let d = b;
            b = status;
            d
        } else {
            status = b;
            t.byte()?
        };
        let chan = b & 0x0F;
        #[expect(clippy::cast_precision_loss, reason = "tick counts are far below 2^52")]
        let qn = |ticks: u64| ticks as f64 / tpq;
        match b & 0xF0 {
            0x90 | 0x80 => {
                let vel = t.byte()?;
                let key = (chan, first);
                if b & 0xF0 == 0x90 && vel > 0 {
                    open.entry(key).or_default().push((tick, vel));
                } else if let Some(stack) = open.get_mut(&key)
                    && !stack.is_empty()
                {
                    let (start, v) = stack.remove(0);
                    notes.push(SmfNote {
                        pitch: first,
                        start: qn(start),
                        end: qn(tick),
                        velocity: v,
                        channel: chan,
                    });
                }
            }
            0xB0 => {
                let val = t.byte()?;
                ccs.push(SmfCc {
                    time: qn(tick),
                    controller: first,
                    value: val,
                    channel: chan,
                });
            }
            0xA0 | 0xE0 => {
                t.byte()?;
            }
            0xC0 | 0xD0 => {}
            _ => return Err(format!("unexpected status byte {b:#04x}")),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(format: u16, tracks: &[Vec<u8>]) -> Vec<u8> {
        let mut f = b"MThd".to_vec();
        f.extend(6u32.to_be_bytes());
        f.extend(format.to_be_bytes());
        f.extend(u16::try_from(tracks.len()).unwrap().to_be_bytes());
        f.extend(480u16.to_be_bytes());
        for t in tracks {
            f.extend(b"MTrk");
            f.extend(u32::try_from(t.len()).unwrap().to_be_bytes());
            f.extend(t);
        }
        f
    }

    #[test]
    fn notes_with_running_status_and_ccs() {
        // C4 at 0 for a beat (note-off as velocity 0, running status), then
        // E4 from beat 1 to 1.5 with a real note-off, and CC1 = 64 at beat 1.
        let track = vec![
            0x00, 0x90, 60, 100, // C4 on
            0x83, 0x60, 60, 0, // +480: C4 off, running status
            0x00, 0xB0, 1, 64, // CC1 64
            0x00, 0x90, 64, 90, // E4 on
            0x81, 0x70, 0x80, 64, 0, // +240: E4 off
            0x00, 0xFF, 0x2F, 0x00,
        ];
        let (notes, ccs) = read(&file(0, &[track])).unwrap();
        assert_eq!(
            notes,
            [
                SmfNote {
                    pitch: 60,
                    start: 0.0,
                    end: 1.0,
                    velocity: 100,
                    channel: 0
                },
                SmfNote {
                    pitch: 64,
                    start: 1.0,
                    end: 1.5,
                    velocity: 90,
                    channel: 0
                },
            ]
        );
        assert_eq!(
            ccs,
            [SmfCc {
                time: 1.0,
                controller: 1,
                value: 64,
                channel: 0
            }]
        );
    }

    #[test]
    fn format_1_tracks_merge() {
        let tempo = vec![
            0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20, 0x00, 0xFF, 0x2F, 0x00,
        ];
        let a = vec![
            0x00, 0x91, 48, 80, 0x83, 0x60, 0x81, 48, 0, 0x00, 0xFF, 0x2F, 0x00,
        ];
        let (notes, _) = read(&file(1, &[tempo, a])).unwrap();
        assert_eq!(
            notes,
            [SmfNote {
                pitch: 48,
                start: 0.0,
                end: 1.0,
                velocity: 80,
                channel: 1
            }]
        );
    }

    #[test]
    fn refuses_what_it_cannot_read() {
        assert!(read(b"RIFF....").unwrap_err().contains("not a MIDI file"));
        assert!(read(&file(2, &[])).unwrap_err().contains("format 2"));
        let mut f = file(0, &[vec![0x00, 0x90, 60]]);
        assert!(read(&f).is_err(), "truncated");
        f.truncate(10);
        assert!(read(&f).is_err());
    }
}
