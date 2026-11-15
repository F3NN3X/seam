//! Rotate windows and Reset sizes on a tiled screen. They change the tree and let
//! `tiled_apply` move the windows, so the one rule (`tiling::may_move`) runs first
//! through `tiled_check`: a window someone else placed floats, and only the windows
//! that may move rotate. A tiled screen's zones are its tiles, in memory, so these
//! can't go through the window worker, which reads the saved layout.

use super::*;
use crate::wincontrol::tile::Back;
use seam::turn::{self, Held};

/// What a turn on a tiled screen did, for the notice and Undo.
pub struct Turned {
    /// Windows that moved, counting each tile once.
    pub moved: usize,
    /// Tiled windows that stayed, because Seam may not move them now.
    pub left: usize,
    /// Each window that moved, and the ones stacked with it, for Undo.
    pub back: Vec<Back>,
    /// A reset's tree before it, which Undo puts back.
    pub tree: Option<Node>,
}

impl Engine {
    /// The tree of a tiled screen on the desktop in front, or why there is none.
    fn turn_tree(&mut self, screen: &str) -> Result<usize, String> {
        if !self.active() {
            return Err("Zones are off, or Seam is standing aside".into());
        }
        if !self.tiling_on(screen) {
            return Err("Smart tiling is off on this screen".into());
        }
        let desktop = self.tiling.desktop.clone();
        Ok(self.tree_index(screen, &desktop))
    }

    /// For Undo: where `key` was, and the windows stacked with it, which go back whole.
    fn turn_back(&self, key: isize, screen: &str, from: usize, rect: Rect, set: Rect, dpi: u32) -> Vec<Back> {
        let members = self.stacks.of(key).map_or_else(|| vec![key], |s| s.members.clone());
        let place =
            |hwnd| Placement { hwnd, target: rect, zone: Some((screen.to_string(), from, from)), dpi, maximize: false };
        members.into_iter().map(|m| Back { place: place(m), set, maximized: false, pulled: None }).collect()
    }

    /// Rotate windows: the windows that may move take the next tile in zone order, or the
    /// previous one. A tile nobody holds, or a window may not move from, is stepped over.
    pub(in crate::snap) fn tiled_rotate(&mut self, screen: &str, forward: bool) -> Result<Turned, String> {
        let i = self.turn_tree(screen)?;
        let (work, dpi, gap) = self.tile_area(screen).ok_or("That screen isn't there any more")?;
        let movable = self.tiled_check(i);
        let tree = &self.tiling.trees[i].tree;
        let rects = tree.rects(work, gap);
        let held: Vec<Held> = (tree.tiles.iter().enumerate())
            .filter_map(|(zone, t)| Some(Held { hwnd: t.hwnd?, zone, stuck: !movable.contains(&t.hwnd?) }))
            .collect();
        let empty: Vec<usize> = (0..tree.tiles.len()).filter(|z| tree.tiles[*z].hwnd.is_none()).collect();
        let moves = turn::rotate(tree.tiles.len(), &held, &empty, forward);
        if moves.is_empty() {
            return Err("Nothing to rotate here".into());
        }
        let mut tiles = tree.tiles.clone();
        let mut back = Vec::new();
        for (key, to) in &moves {
            let Some(from) = tree.zone_of(*key) else { continue };
            tiles[*to] = tree.tiles[from];
            back.extend(self.turn_back(*key, screen, from, rects[from], rects[*to], dpi));
        }
        self.tiling.trees[i].tree.tiles = tiles;
        let left = held.len() - moves.len();
        log::info!("tiling: rotated {} tiled windows, {left} stayed", moves.len());
        self.tiled_apply(i);
        Ok(Turned { moved: moves.len(), left, back, tree: None })
    }

    /// Reset sizes: the tree's splits are even again, and the windows that may move follow.
    pub(in crate::snap) fn tiled_reset(&mut self, screen: &str) -> Result<Turned, String> {
        let i = self.turn_tree(screen)?;
        let (work, dpi, gap) = self.tile_area(screen).ok_or("That screen isn't there any more")?;
        let movable = self.tiled_check(i);
        let tree = &self.tiling.trees[i].tree;
        let even = tree.evened();
        if even == *tree {
            return Err("The splits are even already".into());
        }
        let (before, after) = (tree.rects(work, gap), even.rects(work, gap));
        let held: Vec<(usize, isize)> = tree.tiles.iter().enumerate().filter_map(|(z, t)| Some((z, t.hwnd?))).collect();
        let (moving, left): (Vec<_>, Vec<_>) = held.into_iter().partition(|(_, k)| movable.contains(k));
        let mut back = Vec::new();
        for (zone, key) in &moving {
            back.extend(self.turn_back(*key, screen, *zone, before[*zone], after[*zone], dpi));
        }
        let was = tree.layout();
        self.tiling.trees[i].tree = even;
        log::info!("tiling: evened out the splits, {} tiled windows followed", moving.len());
        self.tiled_apply(i);
        Ok(Turned { moved: moving.len(), left: left.len(), back, tree: Some(was) })
    }

    /// Undo of a reset: the tree's splits are as they were, if its tiles still are.
    pub(in crate::snap) fn tiled_restore(&mut self, screen: &str, node: Node) {
        let desktop = self.tiling.desktop.clone();
        let Some(i) = self.tiling.trees.iter().position(|t| t.screen == screen && t.desktop == desktop) else { return };
        if zones::leaf_count(&node) == self.tiling.trees[i].tree.tiles.len() {
            self.tiling.trees[i].tree.node = Some(node);
            self.tiled_sync();
        }
    }
}
