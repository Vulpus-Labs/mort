//! Reading and replacing one FX's state inside a REAPER track chunk.
//!
//! REAPER has no call for one FX instance's state. A track's state chunk
//! holds its FX chain as a `<FXCHAIN` block, in which each instance is a
//! child block (`<VST`, `<CLAP`, `<JS`, `<AU`, …) followed by lines such as
//! `FXID {guid}`. An instance's state is its child block, found by the
//! `FXID` line after it.

/// The line range `[start, end]` of the FX block that belongs to `guid`.
fn find(lines: &[&str], guid: &str) -> Option<(usize, usize)> {
    let mut depth = 0usize;
    let mut chain_depth: Option<usize> = None;
    let mut last_block: Option<(usize, usize)> = None;
    let mut open: Option<usize> = None;
    for (i, raw) in lines.iter().enumerate() {
        let line = raw.trim_start();
        if line.starts_with('<') {
            depth += 1;
            if line.starts_with("<FXCHAIN") && chain_depth.is_none() {
                chain_depth = Some(depth);
            } else if chain_depth.is_some_and(|c| depth == c + 1) {
                open = Some(i);
            }
        } else if line == ">" {
            if chain_depth.is_some_and(|c| depth == c + 1)
                && let Some(s) = open.take()
            {
                last_block = Some((s, i));
            }
            if chain_depth == Some(depth) {
                chain_depth = None;
            }
            depth = depth.saturating_sub(1);
        } else if chain_depth.is_some_and(|c| depth == c)
            && let Some(rest) = line.strip_prefix("FXID ")
            && rest.trim().eq_ignore_ascii_case(guid)
        {
            return last_block;
        }
    }
    None
}

/// The state block of the FX with `guid` in a track chunk.
#[must_use]
pub fn fx_state(chunk: &str, guid: &str) -> Option<String> {
    let lines: Vec<&str> = chunk.lines().collect();
    let (s, e) = find(&lines, guid)?;
    Some(
        lines[s..=e]
            .iter()
            .map(|l| l.trim_start())
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// The block's kind: its first word without `<`, such as `VST` or `CLAP`.
fn kind(block: &str) -> Option<&str> {
    block
        .trim_start()
        .strip_prefix('<')?
        .split_whitespace()
        .next()
}

/// Why a state could not be put back.
#[derive(Debug, PartialEq, Eq)]
pub enum ReplaceError {
    /// No FX with that GUID in the chunk.
    NotFound,
    /// The new state is not a block of the same kind as the old one.
    WrongKind {
        /// The kind in the project.
        have: String,
        /// The kind given.
        given: String,
    },
}

/// The chunk with the FX's state block replaced by `state`.
///
/// # Errors
///
/// If the FX is not in the chunk, or `state` is not the same kind of block.
pub fn replace_fx_state(chunk: &str, guid: &str, state: &str) -> Result<String, ReplaceError> {
    let lines: Vec<&str> = chunk.lines().collect();
    let (s, e) = find(&lines, guid).ok_or(ReplaceError::NotFound)?;
    let have = kind(lines[s]).unwrap_or("").to_owned();
    let given = kind(state).unwrap_or("").to_owned();
    let balanced = state
        .lines()
        .filter(|l| l.trim_start().starts_with('<'))
        .count()
        == state.lines().filter(|l| l.trim() == ">").count();
    if have != given || !balanced || !state.trim_end().ends_with('>') {
        return Err(ReplaceError::WrongKind { have, given });
    }
    let mut out: Vec<&str> = Vec::with_capacity(lines.len());
    out.extend_from_slice(&lines[..s]);
    out.extend(state.lines());
    out.extend_from_slice(&lines[e + 1..]);
    let mut text = out.join("\n");
    text.push('\n');
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHUNK: &str = "<TRACK
  NAME Bass
  <FXCHAIN
    SHOW 0
    BYPASS 0 0 0
    <VST \"VST: ReaEQ (Cockos)\" reaeq.vst.dylib 0 \"\" 1919247729<56535472656571726561657100000000>
      cWVlcu5e7f4CAAAAAQAAAAAAAAACAAAAAAAAAAIAAAABAAAAAAAAAAIAAAAAAAAAPAAAAAAAAAAAABAAIgAAAAMAAAACAAAA
      AAAQAAAAAAAAAAAA
    >
    FLOATPOS 0 0 0 0
    FXID {11111111-1111-1111-1111-111111111111}
    WAK 0 0
    BYPASS 1 0 0
    <CLAP \"CLAP: mort probe (Vulpus Labs)\" com.vulpuslabs.mort.probe \"\"
      CFG 4 0 0 \"\"
      <STATE
        AAAA
      >
    >
    FXID {22222222-2222-2222-2222-222222222222}
    WAK 0 0
  >
>
";

    #[test]
    fn finds_each_fx_block() {
        let eq = fx_state(CHUNK, "{11111111-1111-1111-1111-111111111111}").unwrap();
        assert!(eq.starts_with("<VST \"VST: ReaEQ (Cockos)\""));
        assert!(eq.ends_with("AAAQAAAAAAAAAAAA\n>"));
        let probe = fx_state(CHUNK, "{22222222-2222-2222-2222-222222222222}").unwrap();
        assert!(probe.starts_with("<CLAP"));
        assert!(
            probe.contains("<STATE\nAAAA\n>"),
            "nested blocks stay inside: {probe}"
        );
        assert_eq!(
            fx_state(CHUNK, "{33333333-3333-3333-3333-333333333333}"),
            None
        );
    }

    #[test]
    fn replaces_a_block_and_keeps_the_rest() {
        let g = "{11111111-1111-1111-1111-111111111111}";
        let new = "<VST \"VST: ReaEQ (Cockos)\" reaeq.vst.dylib 0 \"\" 1919247729<56535472656571726561657100000000>\nBBBB\n>";
        let out = replace_fx_state(CHUNK, g, new).unwrap();
        assert_eq!(fx_state(&out, g).unwrap(), new);
        assert!(out.contains("FXID {22222222-2222-2222-2222-222222222222}"));
        assert_eq!(
            fx_state(&out, "{22222222-2222-2222-2222-222222222222}"),
            fx_state(CHUNK, "{22222222-2222-2222-2222-222222222222}")
        );
        assert_eq!(out.lines().count(), CHUNK.lines().count() - 1);
    }

    #[test]
    fn refuses_another_kind_or_a_broken_block() {
        let g = "{11111111-1111-1111-1111-111111111111}";
        assert_eq!(
            replace_fx_state(CHUNK, g, "<CLAP x\n>"),
            Err(ReplaceError::WrongKind {
                have: "VST".into(),
                given: "CLAP".into()
            })
        );
        assert!(replace_fx_state(CHUNK, g, "<VST x\nAAAA").is_err());
        assert_eq!(
            replace_fx_state(CHUNK, "{33333333-3333-3333-3333-333333333333}", "<VST x\n>"),
            Err(ReplaceError::NotFound)
        );
    }
}
