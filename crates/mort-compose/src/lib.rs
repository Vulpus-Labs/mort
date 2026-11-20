//! Composition for mort (ADR 0013): a song sheet's harmony, voicing,
//! articulation and performance, turned into MIDI events. Nothing here
//! needs REAPER.
//!
//! This crate holds the harmony layer and voicing vocabularies so far:
//! - [`pitch`]: pitch classes, spelled note names and MIDI pitches;
//! - [`scale`]: the scales and modes harmony is built from;
//! - [`chord`]: the chord-symbol grammar;
//! - [`harmony`]: what may sound at a moment;
//! - [`sheet`]: the song sheet's form and harmony;
//! - [`voicing`]: voicing vocabularies and the candidate voicings they
//!   build for a harmony.

pub mod chord;
pub mod harmony;
pub mod pitch;
pub mod scale;
pub mod sheet;
mod source;
pub mod voicing;
