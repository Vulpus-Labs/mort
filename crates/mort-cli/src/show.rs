//! Text output for reads: compact, aligned, made for reading (ADR 0009 §2).

use std::collections::HashMap;

use serde_json::Value;

/// A linear gain as dB, to one decimal.
#[must_use]
pub fn db(gain: f64) -> String {
    if gain <= 0.0 {
        "-inf dB".into()
    } else {
        format!("{:+.1} dB", 20.0 * gain.log10()).replace("+0.0", "0.0")
    }
}

/// A pan from −1 to 1 as `C`, `L20` or `R35`.
#[must_use]
pub fn pan(p: f64) -> String {
    let n = (p * 100.0).round();
    if n == 0.0 {
        "C".into()
    } else if n < 0.0 {
        format!("L{}", -n)
    } else {
        format!("R{n}")
    }
}

/// Lays out rows in columns separated by two spaces; the last column is
/// not padded.
#[must_use]
pub fn table(rows: &[Vec<String>]) -> Vec<String> {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..cols)
        .map(|c| {
            rows.iter()
                .filter_map(|r| r.get(c))
                .map(|s| s.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    rows.iter()
        .map(|r| {
            let mut line = String::new();
            for (c, cell) in r.iter().enumerate() {
                if c + 1 == r.len() {
                    line.push_str(cell);
                } else {
                    line.push_str(cell);
                    line.push_str(&" ".repeat(widths[c] - cell.chars().count() + 2));
                }
            }
            line.trim_end().to_owned()
        })
        .collect()
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

/// The `mort tracks` listing.
#[must_use]
pub fn tracks(list: &Value) -> Vec<String> {
    let tracks = list["tracks"].as_array().cloned().unwrap_or_default();
    let names: HashMap<&str, &str> = tracks
        .iter()
        .chain(std::iter::once(&list["master"]))
        .map(|t| (s(&t["id"]), s(&t["name"])))
        .collect();
    let parents: HashMap<&str, &str> = tracks
        .iter()
        .filter_map(|t| t["parent"].as_str().map(|p| (s(&t["id"]), p)))
        .collect();
    let depth = |id: &str| {
        let mut d = 0;
        let mut at = id;
        while let Some(p) = parents.get(at) {
            d += 1;
            at = p;
        }
        d
    };
    let mut rows = vec![vec![
        "#".to_owned(),
        "name".into(),
        "volume".into(),
        "pan".into(),
        String::new(),
        "fx / sends".into(),
    ]];
    for t in tracks.iter().chain(std::iter::once(&list["master"])) {
        let number = t["index"]
            .as_u64()
            .map_or_else(String::new, |i| (i + 1).to_string());
        let mut flags = String::new();
        if t["mute"] == true {
            flags.push('M');
        }
        if t["solo"] == true {
            flags.push('S');
        }
        if t["main_send"] == false {
            flags.push('x');
        }
        let mut extra: Vec<String> = t["fx"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|f| {
                let n = s(&f["name"]).to_owned();
                if f["enabled"] == false {
                    format!("({n})")
                } else {
                    n
                }
            })
            .collect();
        extra.extend(t["sends"].as_array().into_iter().flatten().map(|snd| {
            format!(
                "→ {} {}",
                names.get(s(&snd["dest"])).copied().unwrap_or("?"),
                db(snd["volume"].as_f64().unwrap_or(0.0))
            )
        }));
        rows.push(vec![
            number,
            format!("{}{}", "  ".repeat(depth(s(&t["id"]))), s(&t["name"])),
            db(t["volume"].as_f64().unwrap_or(0.0)),
            pan(t["pan"].as_f64().unwrap_or(0.0)),
            flags,
            extra.join(", "),
        ]);
    }
    let mut out = table(&rows);
    out.push(String::new());
    out.push("flags: M muted, S soloed, x not sent to its parent; (fx) bypassed".into());
    out
}

/// The `mort fx` listing for one track.
#[must_use]
pub fn fx(track: &Value) -> Vec<String> {
    let chain = track["fx"].as_array().cloned().unwrap_or_default();
    let mut out = vec![format!("FX on {} ({})", s(&track["name"]), chain.len())];
    let rows: Vec<Vec<String>> = chain
        .iter()
        .map(|f| {
            vec![
                format!("{}", f["index"].as_u64().unwrap_or(0) + 1),
                s(&f["name"]).to_owned(),
                if f["enabled"] == false {
                    "bypassed".into()
                } else {
                    String::new()
                },
                s(&f["plugin"]).to_owned(),
            ]
        })
        .collect();
    out.extend(table(&rows).into_iter().map(|l| format!("  {l}")));
    out
}

/// The `mort params` listing, optionally only names containing `filter`.
#[must_use]
pub fn params(result: &Value, track: &str, filter: Option<&str>) -> Vec<String> {
    let f = filter.map(str::to_lowercase);
    let mut rows = vec![vec![
        "#".to_owned(),
        "name".into(),
        "value".into(),
        "display".into(),
    ]];
    let all = result["params"].as_array().cloned().unwrap_or_default();
    let shown: Vec<&Value> = all
        .iter()
        .filter(|p| {
            f.as_ref()
                .is_none_or(|f| s(&p["name"]).to_lowercase().contains(f))
        })
        .collect();
    for p in &shown {
        rows.push(vec![
            p["index"].to_string(),
            s(&p["name"]).to_owned(),
            format!("{:.3}", p["value"].as_f64().unwrap_or(0.0)),
            s(&p["display"]).to_owned(),
        ]);
    }
    let mut out = vec![format!(
        "{} on {}: {} of {} parameters",
        s(&result["fx"]["name"]),
        track,
        shown.len(),
        all.len()
    )];
    out.extend(table(&rows).into_iter().map(|l| format!("  {l}")));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_and_pans() {
        assert_eq!(db(1.0), "0.0 dB");
        assert_eq!(db(0.5), "-6.0 dB");
        assert_eq!(db(2.0), "+6.0 dB");
        assert_eq!(db(0.0), "-inf dB");
        assert_eq!(pan(0.0), "C");
        assert_eq!(pan(-0.2), "L20");
        assert_eq!(pan(0.354), "R35");
    }

    #[test]
    fn table_aligns() {
        let t = table(&[
            vec!["a".into(), "bb".into(), "c".into()],
            vec!["aaa".into(), "b".into(), String::new()],
        ]);
        assert_eq!(t, ["a    bb  c", "aaa  b"]);
    }
}
