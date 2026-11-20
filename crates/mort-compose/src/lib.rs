//! Composition for mort (ADR 0013): a song sheet's harmony, voicing,
//! articulation and performance, turned into MIDI events. Nothing here
//! needs REAPER.
//!
//! This crate holds the harmony and voicing layers so far:
//! - [`pitch`]: pitch classes, spelled note names and MIDI pitches;
//! - [`scale`]: the scales and modes harmony is built from;
//! - [`chord`]: the chord-symbol grammar;
//! - [`harmony`]: what may sound at a moment;
//! - [`sheet`]: the song sheet's form, harmony and parts;
//! - [`part`]: how each part is voiced;
//! - [`lead`]: voice-leading, which turns a part into labelled blocks;
//! - [`voicing`]: voicing vocabularies and the candidate voicings they
//!   build for a harmony.

pub mod chord;
pub mod harmony;
pub mod lead;
pub mod part;
pub mod pitch;
pub mod scale;
pub mod sheet;
mod source;
pub mod voicing;
