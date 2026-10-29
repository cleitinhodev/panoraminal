//! Integer rectangles and dirty-region bookkeeping.

/// Axis-aligned integer rectangle (`x`,`y` top-left; `w`,`h` size).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct IRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl IRect {
    pub const fn new(x: u32, y: u32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }
    pub fn full(w: u32, h: u32) -> Self {
        Self::new(0, 0, w, h)
    }
    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }
    pub fn right(&self) -> u32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> u32 {
        self.y + self.h
    }
    /// Builds a rect from float bounds, clamped to `[0,W]×[0,H]`.
    pub fn from_f32_clamped(x0: f32, y0: f32, x1: f32, y1: f32, w: u32, h: u32) -> Self {
        let cx0 = x0.floor().clamp(0.0, w as f32) as u32;
        let cy0 = y0.floor().clamp(0.0, h as f32) as u32;
        let cx1 = x1.ceil().clamp(0.0, w as f32) as u32;
        let cy1 = y1.ceil().clamp(0.0, h as f32) as u32;
        Self::new(cx0, cy0, cx1.saturating_sub(cx0), cy1.saturating_sub(cy0))
    }
    pub fn union(&self, o: &IRect) -> IRect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        let x0 = self.x.min(o.x);
        let y0 = self.y.min(o.y);
        let x1 = self.right().max(o.right());
        let y1 = self.bottom().max(o.bottom());
        IRect::new(x0, y0, x1 - x0, y1 - y0)
    }
    pub fn intersect(&self, o: &IRect) -> IRect {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = self.right().min(o.right());
        let y1 = self.bottom().min(o.bottom());
        if x1 <= x0 || y1 <= y0 {
            IRect::default()
        } else {
            IRect::new(x0, y0, x1 - x0, y1 - y0)
        }
    }
    pub fn area(&self) -> u64 {
        self.w as u64 * self.h as u64
    }
}

/// A small list of dirty rectangles. Rectangles are merged when the list grows,
/// so that GPU uploads stay few and cheap (a seam-wrapping stroke produces two
/// distant rects instead of one full-width rect).
#[derive(Clone, Debug, Default)]
pub struct DirtyRegion {
    rects: Vec<IRect>,
    full: bool,
}

impl DirtyRegion {
    const MAX: usize = 6;
    pub fn mark_full(&mut self) {
        self.full = true;
        self.rects.clear();
    }
    pub fn is_full(&self) -> bool {
        self.full
    }
    pub fn is_clean(&self) -> bool {
        !self.full && self.rects.is_empty()
    }
    pub fn add(&mut self, r: IRect) {
        if self.full || r.is_empty() {
            return;
        }
        // Merge with an overlapping/nearby rect when cheap.
        for e in self.rects.iter_mut() {
            let u = e.union(&r);
            if u.area() <= e.area() + r.area() + (e.area().min(r.area()) / 2) {
                *e = u;
                return;
            }
        }
        self.rects.push(r);
        if self.rects.len() > Self::MAX {
            // Merge the pair whose union grows least.
            let mut best = (0, 1, u64::MAX);
            for i in 0..self.rects.len() {
                for j in i + 1..self.rects.len() {
                    let u = self.rects[i].union(&self.rects[j]).area();
                    if u < best.2 {
                        best = (i, j, u);
                    }
                }
            }
            let b = self.rects.remove(best.1);
            self.rects[best.0] = self.rects[best.0].union(&b);
        }
    }
    /// Takes the pending rects (or `None` meaning "everything").
    pub fn take(&mut self) -> Option<Vec<IRect>> {
        if self.full {
            self.full = false;
            self.rects.clear();
            None
        } else {
            Some(std::mem::take(&mut self.rects))
        }
    }
    pub fn bounds(&self) -> Option<IRect> {
        self.rects.iter().copied().reduce(|a, b| a.union(&b))
    }
    pub fn merge_from(&mut self, o: &DirtyRegion) {
        if o.full {
            self.mark_full();
        } else {
            for r in &o.rects {
                self.add(*r);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn union_intersect() {
        let a = IRect::new(0, 0, 10, 10);
        let b = IRect::new(5, 5, 10, 10);
        assert_eq!(a.union(&b), IRect::new(0, 0, 15, 15));
        assert_eq!(a.intersect(&b), IRect::new(5, 5, 5, 5));
        assert!(a.intersect(&IRect::new(20, 20, 2, 2)).is_empty());
    }
    #[test]
    fn clamped() {
        let r = IRect::from_f32_clamped(-5.5, 2.2, 3000.0, 9.1, 2048, 1024);
        assert_eq!(r, IRect::new(0, 2, 2048, 8));
    }
    #[test]
    fn dirty_keeps_seam_rects_apart() {
        let mut d = DirtyRegion::default();
        d.add(IRect::new(0, 100, 20, 20));
        d.add(IRect::new(2030, 100, 18, 20));
        let v = d.take().unwrap();
        assert_eq!(v.len(), 2);
        let mut d = DirtyRegion::default();
        for i in 0..50 {
            d.add(IRect::new(i * 40, 0, 10, 10));
        }
        assert!(d.take().unwrap().len() <= DirtyRegion::MAX);
    }
}
