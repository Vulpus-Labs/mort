//! Helpers for reading the crate's KDL files: errors that give the line
//! and column, and a node's arguments and properties.

use kdl::{KdlDocument, KdlEntry, KdlNode};

/// The line and column, from 1, of byte `offset` in `src`.
pub(crate) fn line_col(src: &str, offset: usize) -> (usize, usize) {
    let before = src.get(..offset).unwrap_or(src);
    (
        before.matches('\n').count() + 1,
        before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1,
    )
}

/// Parses KDL, or gives the offset and message of the first problem.
pub(crate) fn parse_kdl(src: &str) -> Result<KdlDocument, (usize, String)> {
    src.parse().map_err(|e: kdl::KdlError| {
        let d = e.diagnostics.first();
        (
            d.map_or(0, |d| d.span.offset()),
            format!(
                "this is not KDL: {}",
                d.and_then(|d| d.message.clone().or_else(|| d.label.clone()))
                    .unwrap_or_else(|| "it does not parse".into())
            ),
        )
    })
}

/// A node's arguments, in order.
pub(crate) fn args(n: &KdlNode) -> Vec<&KdlEntry> {
    n.entries().iter().filter(|e| e.name().is_none()).collect()
}

/// A node's properties, by name, in order.
pub(crate) fn props(n: &KdlNode) -> Vec<(&str, &KdlEntry)> {
    n.entries()
        .iter()
        .filter_map(|e| e.name().map(|k| (k.value(), e)))
        .collect()
}
