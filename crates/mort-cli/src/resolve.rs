//! Turning what people type into ids (ADR 0003 §2).
//!
//! A track can be named by its GUID, its number in REAPER's track list
//! (from 1), `master`, its exact name (ignoring case), or a piece of its
//! name that matches only one track. FX in a chain work the same way, by
//! GUID, position (from 1) or name. Ambiguous names fail and list the
//! matches.

use serde_json::Value;

use crate::client::Failure;

fn describe(v: &Value, label: &str) -> String {
    let n = v["index"]
        .as_u64()
        .map_or_else(|| "-".to_owned(), |i| (i + 1).to_string());
    format!(
        "{n} {} {}",
        v[label].as_str().unwrap_or("?"),
        v["id"].as_str().unwrap_or("")
    )
}

/// Picks one element from `items` by `query`.
///
/// # Errors
///
/// [`Failure::Local`] when nothing matches or more than one thing does.
pub fn pick<'a>(
    items: &'a [Value],
    query: &str,
    what: &str,
    label: &str,
) -> Result<&'a Value, Failure> {
    let q = query.trim();
    let ql = q.to_lowercase();
    if q.starts_with('{') {
        return items
            .iter()
            .find(|v| {
                v["id"]
                    .as_str()
                    .is_some_and(|id| id.eq_ignore_ascii_case(q))
            })
            .ok_or_else(|| Failure::Local(format!("no {what} has id {q}")));
    }
    if let Ok(n) = q.parse::<u64>() {
        return items
            .iter()
            .find(|v| v["index"].as_u64().map(|i| i + 1) == Some(n))
            .ok_or_else(|| Failure::Local(format!("there is no {what} number {n}")));
    }
    let name = |v: &&Value| v[label].as_str().unwrap_or("").to_lowercase();
    let exact: Vec<&Value> = items.iter().filter(|v| name(v) == ql).collect();
    let matches = if exact.is_empty() {
        items.iter().filter(|v| name(v).contains(&ql)).collect()
    } else {
        exact
    };
    match matches.as_slice() {
        [one] => Ok(one),
        [] => {
            let names: Vec<String> = items.iter().map(|v| describe(v, label)).collect();
            Err(Failure::Local(format!(
                "no {what} matches `{q}`. There are:\n  {}",
                if names.is_empty() {
                    "(none)".to_owned()
                } else {
                    names.join("\n  ")
                }
            )))
        }
        many => {
            let names: Vec<String> = many.iter().map(|v| describe(v, label)).collect();
            Err(Failure::Local(format!(
                "`{q}` matches {} {what}s; use a number or id:\n  {}",
                many.len(),
                names.join("\n  ")
            )))
        }
    }
}

/// Picks a track from a `tracks.list` result. The master is named only by
/// `master` or its id, so pieces of names never match it.
///
/// # Errors
///
/// As [`pick`].
pub fn track<'a>(list: &'a Value, query: &str) -> Result<&'a Value, Failure> {
    let q = query.trim();
    let master = &list["master"];
    if q.eq_ignore_ascii_case("master")
        || master["id"]
            .as_str()
            .is_some_and(|id| id.eq_ignore_ascii_case(q))
    {
        return Ok(master);
    }
    let tracks = list["tracks"].as_array().map_or(&[][..], Vec::as_slice);
    pick(tracks, q, "track", "name")
}

/// Picks an FX instance from a track.
///
/// # Errors
///
/// As [`pick`].
pub fn fx<'a>(track: &'a Value, query: &str) -> Result<&'a Value, Failure> {
    let chain = track["fx"].as_array().map_or(&[][..], Vec::as_slice);
    pick(chain, query, "FX", "name")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn list() -> Value {
        json!({
            "master": { "id": "{00000000-0000-0000-0000-000000000001}", "index": null, "name": "MASTER" },
            "tracks": [
                { "id": "{00000000-0000-0000-0000-000000000002}", "index": 0, "name": "Bass" },
                { "id": "{00000000-0000-0000-0000-000000000003}", "index": 1, "name": "Bass DI" },
                { "id": "{00000000-0000-0000-0000-000000000004}", "index": 2, "name": "Pad" },
            ]
        })
    }

    fn name(v: &Value) -> &str {
        v["name"].as_str().unwrap()
    }

    #[test]
    fn by_number_name_piece_id_and_master() {
        let l = list();
        assert_eq!(name(track(&l, "2").unwrap()), "Bass DI");
        assert_eq!(
            name(track(&l, "bass").unwrap()),
            "Bass",
            "exact name wins over pieces"
        );
        assert_eq!(name(track(&l, "di").unwrap()), "Bass DI");
        assert_eq!(
            name(track(&l, "{00000000-0000-0000-0000-000000000004}").unwrap()),
            "Pad"
        );
        assert_eq!(name(track(&l, "Master").unwrap()), "MASTER");
    }

    #[test]
    fn ambiguous_and_missing_list_the_options() {
        let l = list();
        let e = track(&l, "as").unwrap_err().to_string();
        assert!(
            e.contains("matches 2 tracks") && e.contains("1 Bass") && e.contains("2 Bass DI"),
            "{e}"
        );
        let e = track(&l, "drums").unwrap_err().to_string();
        assert!(
            e.contains("no track matches `drums`") && e.contains("3 Pad"),
            "{e}"
        );
        assert!(
            track(&l, "9")
                .unwrap_err()
                .to_string()
                .contains("no track number 9")
        );
    }
}
