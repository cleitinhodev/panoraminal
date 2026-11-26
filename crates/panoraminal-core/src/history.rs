//! Undo / redo.
//!
//! Every [`Edit`] is *self-inverting*: [`Edit::swap`] exchanges the stored state with
//! the document's current state, so the same call performs both undo and redo. Pixel
//! edits keep only the touched 64×64 tiles, which keeps memory low for brush strokes.

use crate::document::{Document, Layer, LayerId, LayerProps};
use crate::rect::IRect;

/// A rectangular block of layer pixels.
#[derive(Clone, Debug)]
pub struct Tile {
    pub rect: IRect,
    pub data: Vec<u8>,
}

impl Tile {
    pub fn capture(px: &[u8], w: u32, rect: IRect) -> Tile {
        let mut data = Vec::with_capacity((rect.w * rect.h * 4) as usize);
        for y in rect.y..rect.bottom() {
            let i = ((y * w + rect.x) * 4) as usize;
            data.extend_from_slice(&px[i..i + (rect.w * 4) as usize]);
        }
        Tile { rect, data }
    }
    /// Swaps the tile contents with the pixels in `px`.
    pub fn swap_into(&mut self, px: &mut [u8], w: u32) {
        let rw = (self.rect.w * 4) as usize;
        for (row, y) in (self.rect.y..self.rect.bottom()).enumerate() {
            let i = ((y * w + self.rect.x) * 4) as usize;
            px[i..i + rw].swap_with_slice(&mut self.data[row * rw..(row + 1) * rw]);
        }
    }
}

#[derive(Clone, Debug)]
pub enum Edit {
    /// Some tiles of one layer changed.
    Pixels { layer: LayerId, tiles: Vec<Tile> },
    /// The full pixel buffer of one layer changed (fill, clear, offset…).
    LayerPixels { layer: LayerId, pixels: Vec<u8> },
    /// A layer was inserted at `index`. `stash` holds it while undone.
    Insert { index: usize, stash: Option<Layer> },
    /// A layer was removed from `index`. `stash` holds it while removed.
    Remove { index: usize, stash: Option<Layer> },
    /// Layer moved from `from` to `to`.
    Move { from: usize, to: usize },
    /// Layer properties changed; `props` holds the "other" state.
    Props { layer: LayerId, props: LayerProps },
    /// The strokes of a line layer changed (1.4+). `art` holds the "other" state;
    /// `None` on either side means "not a line layer" (rasterized), in which case the
    /// pixels stay as they are. Pixels are re-rendered from the strokes, so no pixel
    /// data needs to be stored.
    Lines {
        layer: LayerId,
        art: Option<crate::lines::LineArt>,
    },
    /// Several edits applied together.
    Group(Vec<Edit>),
}

impl Edit {
    /// Applies the inverse of the edit and stores the replaced state (undo ⇄ redo).
    pub fn swap(&mut self, doc: &mut Document) {
        match self {
            Edit::Pixels { layer, tiles } => {
                let w = doc.width;
                let id = *layer;
                let mut rects = Vec::new();
                if let Some(l) = doc.layer_mut(id) {
                    for t in tiles.iter_mut() {
                        t.swap_into(&mut l.pixels, w);
                        rects.push(t.rect);
                    }
                }
                for r in rects {
                    doc.mark_dirty(id, r);
                }
            }
            Edit::LayerPixels { layer, pixels } => {
                let id = *layer;
                if let Some(l) = doc.layer_mut(id) {
                    std::mem::swap(&mut l.pixels, pixels);
                }
                doc.mark_layer_full(id);
            }
            Edit::Insert { index, stash } => {
                if let Some(l) = stash.take() {
                    let id = l.id;
                    doc.layers.insert((*index).min(doc.layers.len()), l);
                    doc.mark_layer_full(id);
                    *self = Edit::Remove {
                        index: *index,
                        stash: None,
                    };
                } else if *index < doc.layers.len() {
                    let l = doc.layers.remove(*index);
                    *self = Edit::Insert {
                        index: *index,
                        stash: Some(l),
                    };
                }
                doc.touch_structure();
            }
            Edit::Remove { index, stash } => {
                // Remove{stash: Some} means "currently removed": swapping re-inserts it.
                if let Some(l) = stash.take() {
                    let id = l.id;
                    doc.layers.insert((*index).min(doc.layers.len()), l);
                    doc.mark_layer_full(id);
                } else if *index < doc.layers.len() {
                    let l = doc.layers.remove(*index);
                    *stash = Some(l);
                }
                doc.touch_structure();
            }
            Edit::Move { from, to } => {
                if *from < doc.layers.len() && *to < doc.layers.len() {
                    let l = doc.layers.remove(*to);
                    doc.layers.insert(*from, l);
                    std::mem::swap(from, to);
                }
                doc.touch_structure();
            }
            Edit::Props { layer, props } => {
                if let Some(cur) = doc.props(*layer) {
                    let other = std::mem::replace(props, cur);
                    doc.set_props(*layer, &other);
                }
            }
            Edit::Lines { layer, art } => {
                let id = *layer;
                let had = doc.layer(id).map(|l| l.is_lines());
                match (had, art.is_some()) {
                    (Some(true), true) => {
                        let cur = doc.layer(id).and_then(|l| l.lines.clone());
                        let new = art.take().unwrap_or_default();
                        crate::lines::apply(doc, id, new);
                        *art = cur;
                    }
                    (Some(_), _) => {
                        if let Some(l) = doc.layer_mut(id) {
                            std::mem::swap(&mut l.lines, art);
                        }
                        doc.touch_structure();
                    }
                    (None, _) => {}
                }
            }
            Edit::Group(v) => {
                // Undo in reverse order; the reversed list then redoes correctly next time.
                for e in v.iter_mut().rev() {
                    e.swap(doc);
                }
                v.reverse();
            }
        }
        doc.fix_active();
    }

    pub fn bytes(&self) -> usize {
        match self {
            Edit::Pixels { tiles, .. } => tiles.iter().map(|t| t.data.len()).sum(),
            Edit::LayerPixels { pixels, .. } => pixels.len(),
            Edit::Insert { stash, .. } | Edit::Remove { stash, .. } => {
                stash.as_ref().map_or(0, |l| {
                    l.pixels.len() + l.lines.as_ref().map_or(0, |a| a.bytes())
                })
            }
            Edit::Lines { art, .. } => art.as_ref().map_or(64, |a| a.bytes()),
            Edit::Group(v) => v.iter().map(|e| e.bytes()).sum(),
            _ => 64,
        }
    }
}

struct Entry {
    edit: Edit,
    /// i18n key describing the action (shown as "Undo: …").
    label: &'static str,
    active_before: LayerId,
    active_after: LayerId,
    /// Unique id of the document state *after* this entry.
    uid: u64,
}

/// Undo/redo stack with memory and length caps.
///
/// Every document state has a unique id ([`History::state_id`]); "modified"
/// means the current state id differs from the one recorded at the last save,
/// which stays correct even if edits happen while a background save runs.
pub struct History {
    entries: Vec<Entry>,
    pos: usize,
    next_uid: u64,
    base_uid: u64,
    saved: Option<u64>,
    bytes: usize,
    pub max_entries: usize,
    pub max_bytes: usize,
}

impl Default for History {
    fn default() -> Self {
        Self::new()
    }
}

impl History {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            pos: 0,
            next_uid: 1,
            base_uid: 0,
            saved: Some(0),
            bytes: 0,
            max_entries: 300,
            max_bytes: 1024 * 1024 * 1024,
        }
    }
    /// Forgets everything; the current state counts as saved.
    pub fn clear(&mut self) {
        *self = Self {
            max_entries: self.max_entries,
            max_bytes: self.max_bytes,
            ..Self::new()
        };
    }
    /// Records an edit that has *already been applied* to the document.
    pub fn push(
        &mut self,
        edit: Edit,
        label: &'static str,
        active_before: LayerId,
        active_after: LayerId,
    ) {
        for e in self.entries.drain(self.pos..) {
            self.bytes -= e.edit.bytes();
        }
        self.bytes += edit.bytes();
        let uid = self.next_uid;
        self.next_uid += 1;
        self.entries.push(Entry {
            edit,
            label,
            active_before,
            active_after,
            uid,
        });
        self.pos += 1;
        while self.entries.len() > 1
            && (self.entries.len() > self.max_entries || self.bytes > self.max_bytes)
        {
            let e = self.entries.remove(0);
            self.bytes -= e.edit.bytes();
            self.base_uid = e.uid;
            self.pos -= 1;
        }
    }
    pub fn can_undo(&self) -> bool {
        self.pos > 0
    }
    pub fn can_redo(&self) -> bool {
        self.pos < self.entries.len()
    }
    pub fn undo_label(&self) -> Option<&'static str> {
        self.pos.checked_sub(1).map(|i| self.entries[i].label)
    }
    pub fn redo_label(&self) -> Option<&'static str> {
        self.entries.get(self.pos).map(|e| e.label)
    }
    pub fn undo(&mut self, doc: &mut Document) -> bool {
        if !self.can_undo() {
            return false;
        }
        self.pos -= 1;
        let e = &mut self.entries[self.pos];
        self.bytes -= e.edit.bytes();
        e.edit.swap(doc);
        self.bytes += e.edit.bytes();
        doc.active = e.active_before;
        doc.fix_active();
        true
    }
    pub fn redo(&mut self, doc: &mut Document) -> bool {
        if !self.can_redo() {
            return false;
        }
        let e = &mut self.entries[self.pos];
        self.bytes -= e.edit.bytes();
        e.edit.swap(doc);
        self.bytes += e.edit.bytes();
        doc.active = e.active_after;
        doc.fix_active();
        self.pos += 1;
        true
    }
    /// Id of the current document state.
    pub fn state_id(&self) -> u64 {
        if self.pos == 0 {
            self.base_uid
        } else {
            self.entries[self.pos - 1].uid
        }
    }
    pub fn mark_saved(&mut self) {
        self.saved = Some(self.state_id());
    }
    /// Marks a state id (taken when a background save started) as saved.
    pub fn mark_saved_state(&mut self, id: u64) {
        self.saved = Some(id);
    }
    /// Forces the document to count as modified (e.g. after recovering an autosave).
    pub fn mark_unsaved(&mut self) {
        self.saved = None;
    }
    pub fn is_modified(&self) -> bool {
        self.saved != Some(self.state_id())
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn position(&self) -> usize {
        self.pos
    }
    pub fn bytes(&self) -> usize {
        self.bytes
    }
}

/// High-level document operations that also record history.
pub mod ops {
    use super::*;

    pub fn add_layer(doc: &mut Document, hist: &mut History, name: &str) -> Option<LayerId> {
        if doc.layers.len() >= crate::MAX_LAYERS {
            return None;
        }
        let before = doc.active;
        let l = doc.new_layer(name);
        let id = l.id;
        let index = doc.active_index() + 1;
        doc.layers.insert(index.min(doc.layers.len()), l);
        doc.active = id;
        doc.touch_structure();
        doc.mark_layer_full(id);
        hist.push(
            Edit::Remove { index, stash: None },
            "hist.add_layer",
            before,
            id,
        );
        Some(id)
    }

    /// Adds an empty vector line layer above the active one.
    pub fn add_line_layer(doc: &mut Document, hist: &mut History, name: &str) -> Option<LayerId> {
        if doc.layers.len() >= crate::MAX_LAYERS {
            return None;
        }
        let before = doc.active;
        let mut l = doc.new_layer(name);
        l.lines = Some(crate::lines::LineArt::default());
        let id = l.id;
        let index = doc.active_index() + 1;
        doc.layers.insert(index.min(doc.layers.len()), l);
        doc.active = id;
        doc.touch_structure();
        doc.mark_layer_full(id);
        hist.push(
            Edit::Remove { index, stash: None },
            "hist.add_line_layer",
            before,
            id,
        );
        Some(id)
    }

    /// Records a line edit that has already been applied with [`crate::lines::apply`];
    /// `before` is the previous strokes. Nothing is recorded if nothing changed.
    pub fn commit_lines(
        doc: &mut Document,
        hist: &mut History,
        id: LayerId,
        before: crate::lines::LineArt,
        label: &'static str,
    ) -> bool {
        let same = doc
            .layer(id)
            .and_then(|l| l.lines.as_ref())
            .is_none_or(|cur| *cur == before);
        if same {
            return false;
        }
        hist.push(
            Edit::Lines {
                layer: id,
                art: Some(before),
            },
            label,
            doc.active,
            doc.active,
        );
        true
    }

    /// Replaces the strokes of a line layer and records it in one step.
    pub fn set_lines(
        doc: &mut Document,
        hist: &mut History,
        id: LayerId,
        art: crate::lines::LineArt,
        label: &'static str,
    ) -> bool {
        let Some(before) = doc.layer(id).and_then(|l| l.lines.clone()) else {
            return false;
        };
        if before == art {
            return false;
        }
        crate::lines::apply(doc, id, art);
        commit_lines(doc, hist, id, before, label)
    }

    /// Turns a line layer into a normal raster layer (keeps its pixels).
    pub fn rasterize_lines(doc: &mut Document, hist: &mut History, id: LayerId) -> bool {
        let Some(l) = doc.layer_mut(id) else {
            return false;
        };
        let Some(art) = l.lines.take() else {
            return false;
        };
        doc.touch_structure();
        hist.push(
            Edit::Lines {
                layer: id,
                art: Some(art),
            },
            "hist.rasterize_lines",
            doc.active,
            doc.active,
        );
        true
    }

    pub fn duplicate_layer(
        doc: &mut Document,
        hist: &mut History,
        suffix: &str,
    ) -> Option<LayerId> {
        if doc.layers.len() >= crate::MAX_LAYERS {
            return None;
        }
        let before = doc.active;
        let idx = doc.active_index();
        let src = doc.layers.get(idx)?.clone();
        let id = doc.alloc_id();
        let mut l = src;
        l.id = id;
        l.name = format!("{} {}", l.name, suffix);
        doc.layers.insert(idx + 1, l);
        doc.active = id;
        doc.touch_structure();
        doc.mark_layer_full(id);
        hist.push(
            Edit::Remove {
                index: idx + 1,
                stash: None,
            },
            "hist.duplicate_layer",
            before,
            id,
        );
        Some(id)
    }

    pub fn delete_layer(doc: &mut Document, hist: &mut History) -> bool {
        if doc.layers.len() <= 1 {
            return false;
        }
        let before = doc.active;
        let idx = doc.active_index();
        let mut e = Edit::Remove {
            index: idx,
            stash: None,
        };
        e.swap(doc);
        doc.active = doc.layers[idx.min(doc.layers.len() - 1)].id;
        hist.push(e, "hist.delete_layer", before, doc.active);
        true
    }

    /// Moves layer `from` to index `to` (drag & drop reordering).
    pub fn move_layer(doc: &mut Document, hist: &mut History, from: usize, to: usize) -> bool {
        let n = doc.layers.len();
        if from >= n || to >= n || from == to {
            return false;
        }
        let l = doc.layers.remove(from);
        doc.layers.insert(to, l);
        doc.touch_structure();
        hist.push(
            Edit::Move { from, to },
            "hist.move_layer",
            doc.active,
            doc.active,
        );
        true
    }

    pub fn merge_down(doc: &mut Document, hist: &mut History) -> bool {
        let idx = doc.active_index();
        let Some(merged) = doc.merged_down_pixels(idx) else {
            return false;
        };
        let before = doc.active;
        let low_id = doc.layers[idx - 1].id;
        let mut px = Edit::LayerPixels {
            layer: low_id,
            pixels: merged,
        };
        px.swap(doc);
        let mut rm = Edit::Remove {
            index: idx,
            stash: None,
        };
        rm.swap(doc);
        let mut edits = vec![px, rm];
        // Merging onto a line layer turns it into a normal layer (its pixels now hold more
        // than the strokes).
        if let Some(art) = doc.layer_mut(low_id).and_then(|l| l.lines.take()) {
            doc.touch_structure();
            edits.push(Edit::Lines {
                layer: low_id,
                art: Some(art),
            });
        }
        doc.active = low_id;
        hist.push(Edit::Group(edits), "hist.merge_down", before, low_id);
        true
    }

    pub fn set_props(
        doc: &mut Document,
        hist: &mut History,
        id: LayerId,
        new: LayerProps,
        label: &'static str,
    ) {
        let Some(old) = doc.props(id) else { return };
        if old == new {
            return;
        }
        doc.set_props(id, &new);
        hist.push(
            Edit::Props {
                layer: id,
                props: old,
            },
            label,
            doc.active,
            doc.active,
        );
    }

    /// Records a full-layer pixel replacement; `old` are the previous pixels
    /// (the new ones must already be in the document).
    pub fn replaced_pixels(
        doc: &mut Document,
        hist: &mut History,
        id: LayerId,
        old: Vec<u8>,
        label: &'static str,
    ) {
        doc.mark_layer_full(id);
        hist.push(
            Edit::LayerPixels {
                layer: id,
                pixels: old,
            },
            label,
            doc.active,
            doc.active,
        );
    }

    /// Shifts every layer horizontally by `dx` pixels (rotates the panorama's heading).
    pub fn offset_all(doc: &mut Document, hist: &mut History, dx: i64) {
        let w = doc.width as i64;
        let dx = dx.rem_euclid(w) as usize;
        if dx == 0 {
            return;
        }
        let mut edits = Vec::new();
        let row = doc.width as usize * 4;
        // Line layers move their strokes (and are re-rendered from them).
        let deg = dx as f64 / doc.width as f64 * 360.0;
        let line_ids: Vec<LayerId> = doc
            .layers
            .iter()
            .filter(|l| l.is_lines())
            .map(|l| l.id)
            .collect();
        for id in &line_ids {
            let Some(before) = doc.layer(*id).and_then(|l| l.lines.clone()) else {
                continue;
            };
            let mut art = before.clone();
            art.offset_lon(deg);
            crate::lines::apply(doc, *id, art);
            edits.push(Edit::Lines {
                layer: *id,
                art: Some(before),
            });
        }
        for l in doc.layers.iter_mut().filter(|l| !l.is_lines()) {
            let old = l.pixels.clone();
            for r in l.pixels.chunks_exact_mut(row) {
                r.rotate_right(dx * 4);
            }
            edits.push(Edit::LayerPixels {
                layer: l.id,
                pixels: old,
            });
        }
        let ids: Vec<_> = doc
            .layers
            .iter()
            .filter(|l| !l.is_lines())
            .map(|l| l.id)
            .collect();
        for id in ids {
            doc.mark_layer_full(id);
        }
        hist.push(Edit::Group(edits), "hist.offset", doc.active, doc.active);
    }
}

#[cfg(test)]
mod tests {
    use super::ops::*;
    use super::*;
    use crate::document::Background;
    use crate::BlendMode;

    fn doc() -> Document {
        Document::new_blank(64, 32, Background::Sky, "bg", "l1")
    }

    #[test]
    fn add_undo_redo() {
        let mut d = doc();
        let mut h = History::new();
        let id = add_layer(&mut d, &mut h, "L2").unwrap();
        assert_eq!(d.layers.len(), 3);
        assert!(h.is_modified());
        h.undo(&mut d);
        assert_eq!(d.layers.len(), 2);
        assert!(d.layer(id).is_none());
        assert!(!h.is_modified());
        h.redo(&mut d);
        assert_eq!(d.layers.len(), 3);
        assert_eq!(d.active, id);
    }

    #[test]
    fn delete_and_restore() {
        let mut d = doc();
        let mut h = History::new();
        let id = d.active;
        d.layer_mut(id).unwrap().pixels[0] = 77;
        delete_layer(&mut d, &mut h);
        assert!(d.layer(id).is_none());
        h.undo(&mut d);
        assert_eq!(d.layer(id).unwrap().pixels[0], 77);
        assert_eq!(d.active, id);
        h.redo(&mut d);
        assert!(d.layer(id).is_none());
    }

    #[test]
    fn move_roundtrip() {
        let mut d = doc();
        let mut h = History::new();
        add_layer(&mut d, &mut h, "L2");
        let ids: Vec<_> = d.layers.iter().map(|l| l.id).collect();
        move_layer(&mut d, &mut h, 2, 0);
        assert_eq!(d.layers[0].id, ids[2]);
        h.undo(&mut d);
        assert_eq!(d.layers.iter().map(|l| l.id).collect::<Vec<_>>(), ids);
        h.redo(&mut d);
        assert_eq!(d.layers[0].id, ids[2]);
    }

    #[test]
    fn merge_undo_restores_pixels() {
        let mut d = doc();
        let mut h = History::new();
        let top = d.active;
        {
            let l = d.layer_mut(top).unwrap();
            l.blend = BlendMode::Multiply;
            for p in l.pixels.chunks_exact_mut(4) {
                p.copy_from_slice(&[255, 0, 0, 255]);
            }
        }
        let flat = d.flatten(None);
        let base_before = d.layers[0].pixels.clone();
        assert!(merge_down(&mut d, &mut h));
        assert_eq!(d.layers.len(), 1);
        assert_eq!(d.flatten(None), flat);
        h.undo(&mut d);
        assert_eq!(d.layers.len(), 2);
        assert_eq!(d.layers[0].pixels, base_before);
        assert_eq!(d.active, top);
        h.redo(&mut d);
        assert_eq!(d.layers.len(), 1);
        assert_eq!(d.flatten(None), flat);
    }

    #[test]
    fn props_undo() {
        let mut d = doc();
        let mut h = History::new();
        let id = d.active;
        let mut p = d.props(id).unwrap();
        p.opacity = 0.3;
        p.blend = BlendMode::Screen;
        set_props(&mut d, &mut h, id, p, "hist.props");
        h.undo(&mut d);
        assert_eq!(d.layer(id).unwrap().opacity, 1.0);
        h.redo(&mut d);
        assert_eq!(d.layer(id).unwrap().blend, BlendMode::Screen);
    }

    #[test]
    fn offset_wraps() {
        let mut d = doc();
        let mut h = History::new();
        let id = d.active;
        d.layer_mut(id).unwrap().pixels[3] = 255; // pixel (0,0)
        offset_all(&mut d, &mut h, 5);
        assert_eq!(d.layer(id).unwrap().pixels[5 * 4 + 3], 255);
        offset_all(&mut d, &mut h, -10);
        assert_eq!(d.layer(id).unwrap().pixels[(64 - 5) * 4 + 3], 255);
        h.undo(&mut d);
        h.undo(&mut d);
        assert_eq!(d.layer(id).unwrap().pixels[3], 255);
    }

    #[test]
    fn memory_cap_drops_oldest() {
        let mut d = doc();
        let mut h = History::new();
        h.max_entries = 3;
        for i in 0..6 {
            add_layer(&mut d, &mut h, &format!("L{i}"));
        }
        assert_eq!(h.len(), 3);
        assert!(h.is_modified());
    }

    #[test]
    fn saved_state_survives_concurrent_edits() {
        let mut d = doc();
        let mut h = History::new();
        add_layer(&mut d, &mut h, "a");
        let at_save = h.state_id();
        add_layer(&mut d, &mut h, "b"); // edit while "saving"
        h.mark_saved_state(at_save);
        assert!(h.is_modified());
        h.undo(&mut d);
        assert!(!h.is_modified());
        // A different branch with the same depth is still "modified".
        add_layer(&mut d, &mut h, "c");
        h.undo(&mut d);
        add_layer(&mut d, &mut h, "d");
        assert!(h.is_modified());
    }

    fn line_art(lon: f64) -> crate::lines::LineArt {
        use crate::lines::{LinePoint, LineStroke};
        crate::lines::LineArt {
            strokes: vec![LineStroke {
                points: vec![
                    LinePoint {
                        lon,
                        lat: 0.0,
                        w: 1.0,
                        sharp: false,
                    },
                    LinePoint {
                        lon: lon + 40.0,
                        lat: 20.0,
                        w: 2.0,
                        sharp: false,
                    },
                ],
                color: [10, 200, 30],
                width: 3.0,
                ..Default::default()
            }],
        }
    }
    fn rendered(d: &Document, id: LayerId) -> Vec<u8> {
        let l = d.layer(id).unwrap();
        let mut px = vec![0; l.pixels.len()];
        crate::lines::render_full(l.lines.as_ref().unwrap(), &mut px, d.width, d.height);
        px
    }

    #[test]
    fn line_edits_undo_and_redo() {
        let mut d = Document::new_blank(256, 128, Background::Sky, "bg", "l1");
        let mut h = History::new();
        let id = add_line_layer(&mut d, &mut h, "Lines").unwrap();
        assert!(d.active_is_lines());
        assert!(d.layer(id).unwrap().is_empty());
        assert!(set_lines(
            &mut d,
            &mut h,
            id,
            line_art(10.0),
            "hist.line_add"
        ));
        assert!(!d.layer(id).unwrap().is_empty());
        let first = d.layer(id).unwrap().pixels.clone();
        // Live edit + commit (like dragging a point).
        let before = d.layer(id).unwrap().lines.clone().unwrap();
        crate::lines::apply(&mut d, id, line_art(100.0));
        assert!(commit_lines(&mut d, &mut h, id, before, "hist.line_edit"));
        assert_eq!(d.layer(id).unwrap().pixels, rendered(&d, id));
        let second = d.layer(id).unwrap().pixels.clone();
        assert_ne!(first, second);
        h.undo(&mut d);
        assert_eq!(d.layer(id).unwrap().pixels, first);
        assert_eq!(d.layer(id).unwrap().lines, Some(line_art(10.0)));
        h.undo(&mut d);
        assert!(d.layer(id).unwrap().is_empty());
        h.redo(&mut d);
        h.redo(&mut d);
        assert_eq!(d.layer(id).unwrap().pixels, second);
        // Unchanged commits are not recorded.
        let n = h.len();
        let same = d.layer(id).unwrap().lines.clone().unwrap();
        assert!(!commit_lines(&mut d, &mut h, id, same, "hist.line_edit"));
        assert_eq!(h.len(), n);
    }

    #[test]
    fn rasterize_and_merge_keep_pixels() {
        let mut d = Document::new_blank(256, 128, Background::Sky, "bg", "l1");
        let mut h = History::new();
        let id = add_line_layer(&mut d, &mut h, "Lines").unwrap();
        set_lines(&mut d, &mut h, id, line_art(30.0), "hist.line_add");
        let px = d.layer(id).unwrap().pixels.clone();
        assert!(rasterize_lines(&mut d, &mut h, id));
        assert!(!d.layer(id).unwrap().is_lines());
        assert_eq!(d.layer(id).unwrap().pixels, px);
        h.undo(&mut d);
        assert!(d.layer(id).unwrap().is_lines());
        // Merging a raster layer down onto the line layer makes it raster.
        let top = add_layer(&mut d, &mut h, "top").unwrap();
        d.layer_mut(top).unwrap().pixels[3] = 255;
        assert!(merge_down(&mut d, &mut h));
        assert!(!d.layer(id).unwrap().is_lines());
        h.undo(&mut d);
        assert!(d.layer(id).unwrap().is_lines());
        assert_eq!(d.layer(id).unwrap().pixels, px);
        // A line layer merged down into a raster layer just adds its pixels.
        d.active = id;
        let before = d.flatten(None);
        assert!(merge_down(&mut d, &mut h));
        assert!(d.layers.iter().all(|l| !l.is_lines()));
        let after = d.flatten(None);
        let diff = before
            .iter()
            .zip(&after)
            .map(|(a, b)| (*a as i32 - *b as i32).abs())
            .max();
        assert!(diff.unwrap() <= 1);
    }

    #[test]
    fn offset_moves_line_strokes() {
        let mut d = Document::new_blank(256, 128, Background::Sky, "bg", "l1");
        let mut h = History::new();
        let id = add_line_layer(&mut d, &mut h, "Lines").unwrap();
        set_lines(&mut d, &mut h, id, line_art(350.0), "hist.line_add");
        let px = d.layer(id).unwrap().pixels.clone();
        offset_all(&mut d, &mut h, 64);
        let l = d.layer(id).unwrap();
        assert!((l.lines.as_ref().unwrap().strokes[0].points[0].lon - 80.0).abs() < 1e-9);
        assert_eq!(l.pixels, rendered(&d, id));
        h.undo(&mut d);
        assert_eq!(d.layer(id).unwrap().pixels, px);
    }
}
