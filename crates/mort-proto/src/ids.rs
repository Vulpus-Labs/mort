//! Typed REAPER GUIDs (ADR 0003 §1).
//!
//! REAPER names tracks, FX, items, takes and envelopes by GUID, written as
//! `{8-4-4-4-12}` uppercase hex. Each kind of element gets its own type, so
//! a track id cannot be passed where an FX id is expected. All of them
//! serialise as REAPER's text form.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A 128-bit GUID, in REAPER's `{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}` form.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Guid(pub u128);

/// Why a string is not a GUID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuidError {
    /// The text that failed to parse.
    pub input: String,
}

impl fmt::Display for GuidError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "`{}` is not a GUID; expected REAPER's form, for example \
             {{0A1B2C3D-4E5F-6071-8293-A4B5C6D7E8F9}}",
            self.input
        )
    }
}

impl std::error::Error for GuidError {}

impl FromStr for Guid {
    type Err = GuidError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || GuidError {
            input: s.to_owned(),
        };
        let inner = s
            .strip_prefix('{')
            .and_then(|r| r.strip_suffix('}'))
            .ok_or_else(err)?;
        let groups: Vec<&str> = inner.split('-').collect();
        let lens = [8, 4, 4, 4, 12];
        if groups.len() != lens.len() || groups.iter().zip(lens).any(|(g, n)| g.len() != n) {
            return Err(err());
        }
        let hex: String = groups.concat();
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(err());
        }
        u128::from_str_radix(&hex, 16).map(Guid).map_err(|_| err())
    }
}

impl fmt::Display for Guid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let h = format!("{:032X}", self.0);
        write!(
            f,
            "{{{}-{}-{}-{}-{}}}",
            &h[0..8],
            &h[8..12],
            &h[12..16],
            &h[16..20],
            &h[20..32]
        )
    }
}

impl fmt::Debug for Guid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl Serialize for Guid {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Guid {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

macro_rules! typed_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Guid);

        impl FromStr for $name {
            type Err = GuidError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                s.parse().map($name)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!(stringify!($name), "({})"), self.0)
            }
        }
    };
}

typed_id!(
    /// A track, including the master track.
    TrackId
);
typed_id!(
    /// An FX instance in a track's chain.
    FxId
);
typed_id!(
    /// A media item.
    ItemId
);
typed_id!(
    /// A take within an item.
    TakeId
);
typed_id!(
    /// An envelope: track volume, pan, a send, or an FX parameter.
    EnvelopeId
);

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "{0A1B2C3D-4E5F-6071-8293-A4B5C6D7E8F9}";

    #[test]
    fn round_trips_text() {
        let g: Guid = TEXT.parse().unwrap();
        assert_eq!(g.0, 0x0A1B_2C3D_4E5F_6071_8293_A4B5_C6D7_E8F9);
        assert_eq!(g.to_string(), TEXT);
    }

    #[test]
    fn accepts_lowercase_and_prints_uppercase() {
        let g: Guid = TEXT.to_lowercase().parse().unwrap();
        assert_eq!(g.to_string(), TEXT);
    }

    #[test]
    fn rejects_malformed() {
        for bad in [
            "",
            "0A1B2C3D-4E5F-6071-8293-A4B5C6D7E8F9",
            "{0A1B2C3D-4E5F-6071-8293-A4B5C6D7E8F}",
            "{0A1B2C3D4E5F-6071-8293-A4B5C6D7E8F9}",
            "{0A1B2C3D-4E5F-6071-8293-A4B5C6D7E8FG}",
            "{+A1B2C3D-4E5F-6071-8293-A4B5C6D7E8F9}",
        ] {
            let e = bad.parse::<Guid>().unwrap_err();
            assert_eq!(e.input, bad);
            assert!(e.to_string().contains("REAPER's form"));
        }
    }

    #[test]
    fn typed_ids_serialise_as_text() {
        let t: TrackId = TEXT.parse().unwrap();
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(json, format!("\"{TEXT}\""));
        let back: TrackId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, t);
        assert_eq!(format!("{t:?}"), format!("TrackId({TEXT})"));
    }

    #[test]
    fn bad_id_in_json_names_the_input() {
        let e = serde_json::from_str::<FxId>("\"nope\"").unwrap_err();
        assert!(e.to_string().contains("`nope` is not a GUID"));
    }
}
