//! Damage tracking for partial repaints across frames.

use std::collections::HashMap;

use rustui_core::{NodeId, Rect, Runtime};

/// Tracks changed node regions between frames to enable partial present / scissor damage.
#[derive(Debug, Default)]
pub struct DamageTracker {
    prev_generations: HashMap<NodeId, (u64, Rect)>,
    dirty_regions: Vec<Rect>,
    full_repaint: bool,
}

impl DamageTracker {
    pub fn new() -> Self {
        Self {
            prev_generations: HashMap::new(),
            dirty_regions: Vec::new(),
            full_repaint: true,
        }
    }

    pub fn invalidate_all(&mut self) {
        self.full_repaint = true;
    }

    /// Scans `runtime` nodes and collects merged bounding boxes of changed nodes.
    pub fn collect(&mut self, runtime: &Runtime, viewport: Rect) -> Vec<Rect> {
        self.dirty_regions.clear();
        let mut current_seen = HashMap::new();

        runtime.with_inner(|inner| {
            for (id, node) in &inner.nodes {
                let bounds = node.bounds();
                let gen_num = node.paint_dirty_generation;
                current_seen.insert(id, (gen_num, bounds));

                match self.prev_generations.get(&id) {
                    None => {
                        if bounds.width > 0.0 && bounds.height > 0.0 {
                            self.dirty_regions.push(bounds);
                        }
                    }
                    Some(&(prev_gen, prev_bounds)) => {
                        if prev_gen != gen_num || prev_bounds != bounds {
                            if prev_bounds.width > 0.0 && prev_bounds.height > 0.0 {
                                self.dirty_regions.push(prev_bounds);
                            }
                            if bounds.width > 0.0 && bounds.height > 0.0 {
                                self.dirty_regions.push(bounds);
                            }
                        }
                    }
                }
            }
        });

        // Any removed node's previous bounding box is also damaged.
        for (old_id, &(_, old_bounds)) in &self.prev_generations {
            if !current_seen.contains_key(old_id)
                && old_bounds.width > 0.0
                && old_bounds.height > 0.0
            {
                self.dirty_regions.push(old_bounds);
            }
        }

        self.prev_generations = current_seen;

        if self.full_repaint {
            self.full_repaint = false;
            return vec![viewport];
        }

        merge_rects(&mut self.dirty_regions, viewport)
    }

    pub fn last_regions(&self) -> &[Rect] {
        &self.dirty_regions
    }
}

fn merge_rects(rects: &mut Vec<Rect>, clamp: Rect) -> Vec<Rect> {
    let mut clipped: Vec<Rect> = rects
        .drain(..)
        .filter_map(|r| r.expand(2.0).intersection(&clamp))
        .collect();
    let mut merged: Vec<Rect> = Vec::new();
    for r in clipped.drain(..) {
        if let Some(existing) = merged.iter_mut().find(|m| m.intersects(&r)) {
            *existing = existing.union(&r);
        } else {
            merged.push(r);
        }
    }
    *rects = merged.clone();
    merged
}
