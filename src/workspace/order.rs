//! The order of a desk's workspaces, which is also their numbering (D691).

use super::*;

/// Why a workspace couldn't be moved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveError {
    /// No workspace of the desk has that id.
    NotFound,
    /// The number is 0 or past the last workspace.
    OutOfRange,
}

/// The saved list with workspace `id` moved to number `to`, counted from 1 among the
/// workspaces this build can read. The entries are moved as they are, never read and
/// written again, so keys an older or newer Seam wrote stay. An entry that can't be read
/// keeps its place beside the others.
pub fn moved_to(saved: &[Value], id: &str, to: usize) -> Result<Vec<Value>, MoveError> {
    let readable = |list: &[Value]| -> Vec<usize> { (0..list.len()).filter(|i| parse(&list[*i]).is_some()).collect() };
    let rows = readable(saved);
    let from =
        rows.iter().copied().find(|i| parse(&saved[*i]).is_some_and(|w| w.id == id)).ok_or(MoveError::NotFound)?;
    if to == 0 || to > rows.len() {
        return Err(MoveError::OutOfRange);
    }
    let mut list = saved.to_vec();
    let moved = list.remove(from);
    let rest = readable(&list);
    // Before the workspace now at that number, or after the last one.
    let at = rest.get(to - 1).copied().unwrap_or_else(|| rest.last().map_or(0, |last| last + 1));
    list.insert(at, moved);
    Ok(list)
}
