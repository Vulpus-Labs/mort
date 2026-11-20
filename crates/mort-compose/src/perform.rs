//! Performance (ADR 0013 §8): swing, timing and velocity, applied last
//! and seeded per part and section so the same sheet always plays the
//! same way.

use std::collections::BTreeMap;

use crate::articulate::Event;
use crate::sheet::Section;

/// A small, fixed random generator (`SplitMix64`), so feel needs no
/// dependency and never changes between builds.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A whole number from `-n` to `n`.
    fn within(&mut self, n: u32) -> i64 {
        if n == 0 {
            return 0;
        }
        let span = u64::from(n) * 2 + 1;
        i64::try_from(self.next() % span).unwrap_or(0) - i64::from(n)
    }
}

fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Applies `feel` (`swing`, `timing`, `velocity`, `seed`) to a section's
/// events, which must be sorted.
pub(crate) fn apply(
    events: &mut [Event],
    section: &Section,
    feel: &BTreeMap<String, f64>,
    part: &str,
) {
    let get = |k: &str| feel.get(k).copied().unwrap_or(0.0);
    let beat = section.meter.beat_ticks();
    let swing = feel.get("swing").copied().unwrap_or(0.5);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "checked ranges"
    )]
    let (timing, spread, seed) = (
        get("timing") as u32,
        get("velocity") as u32,
        get("seed") as u64,
    );
    let mut rng = Rng(fnv(&format!("{part}/{}/{seed}", section.name)));
    for e in events.iter_mut() {
        if (swing - 0.5).abs() > f64::EPSILON && e.start % beat == beat / 2 {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "within a beat"
            )]
            let late = (swing * f64::from(beat)).round() as u32;
            let shift = late - beat / 2;
            e.start += shift;
            e.length = e.length.saturating_sub(shift).max(1);
        }
        let dt = rng.within(timing);
        let dv = rng.within(spread);
        let start =
            (i64::from(e.start) + dt).clamp(0, i64::from(section.length().saturating_sub(1)));
        e.start = u32::try_from(start).unwrap_or(e.start);
        e.velocity = u8::try_from((i64::from(e.velocity) + dv).clamp(1, 127)).unwrap_or(e.velocity);
    }
}
