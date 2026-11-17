//! Debug inspector overlay showing node tree bounds, dirty regions, and signal graph stats.

use rustui_core::{Color, CornerRadii, Rect, Runtime};

use crate::damage::DamageTracker;
use crate::display_list::DisplayList;

/// Renders node layout boxes, damaged dirty regions, and signal metrics on top of a frame.
#[derive(Debug, Default, Clone, Copy)]
pub struct InspectorOverlay {
    pub enabled: bool,
    pub show_bounds: bool,
    pub show_dirty_regions: bool,
    pub show_stats_badge: bool,
}

impl InspectorOverlay {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            show_bounds: true,
            show_dirty_regions: true,
            show_stats_badge: true,
        }
    }

    pub fn emit(
        &self,
        runtime: &Runtime,
        damage: &DamageTracker,
        list: &mut DisplayList,
        viewport: Rect,
    ) {
        if !self.enabled {
            return;
        }

        if self.show_bounds {
            runtime.with_inner(|inner| {
                for (_, node) in &inner.nodes {
                    let b = node.bounds();
                    if b.width <= 0.0 || b.height <= 0.0 {
                        continue;
                    }
                    list.stroke_rounded_rect(
                        b,
                        CornerRadii::ZERO,
                        1.0,
                        Color::rgba(0.0, 0.85, 0.90, 0.55),
                    );
                }
            });
        }

        if self.show_dirty_regions {
            for region in damage.last_regions() {
                list.fill_rounded_rect(
                    *region,
                    CornerRadii::ZERO,
                    Color::rgba(0.96, 0.16, 0.58, 0.14),
                );
                list.stroke_rounded_rect(
                    *region,
                    CornerRadii::ZERO,
                    1.5,
                    Color::rgba(0.96, 0.16, 0.58, 0.85),
                );
            }
        }

        if self.show_stats_badge {
            let badge_rect = Rect::new((viewport.max_x() - 220.0).max(8.0), 8.0, 212.0, 52.0);
            list.fill_rounded_rect(
                badge_rect,
                CornerRadii::all(6.0),
                Color::rgba(0.06, 0.08, 0.12, 0.88),
            );
            list.stroke_rounded_rect(
                badge_rect,
                CornerRadii::all(6.0),
                1.0,
                Color::rgba(0.35, 0.72, 1.0, 0.85),
            );
        }
    }
}
