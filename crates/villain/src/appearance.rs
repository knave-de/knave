//! Compositor geometry for the Knave-owned appearance contract.
use knave_config::{
    WindowAppearance,
    appearance::{Edges, ViewEffects},
};
use smithay::utils::{Logical, Rectangle};
type Geometry = Rectangle<i32, Logical>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TitlebarControl {
    Close,
    Maximize,
    Float,
    Minimize,
}

impl TitlebarControl {
    pub(crate) const fn right_slot(self) -> i32 {
        match self {
            Self::Close => 0,
            Self::Maximize => 1,
            Self::Float => 2,
            Self::Minimize => 3,
        }
    }

    pub(crate) fn from_right_distance(distance: f64) -> Option<Self> {
        if !(distance > 0.0 && distance <= 96.0) {
            return None;
        }
        Some(if distance <= 24.0 {
            Self::Close
        } else if distance <= 48.0 {
            Self::Maximize
        } else if distance <= 72.0 {
            Self::Float
        } else {
            Self::Minimize
        })
    }
}

pub(crate) fn inset(mut rect: Geometry, edges: Edges) -> Geometry {
    let horizontal = fit_pair(rect.size.w, i32::from(edges.left), i32::from(edges.right));
    let vertical = fit_pair(rect.size.h, i32::from(edges.top), i32::from(edges.bottom));
    rect.loc.x += horizontal.0;
    rect.loc.y += vertical.0;
    rect.size.w = (rect.size.w - horizontal.0 - horizontal.1).max(1);
    rect.size.h = (rect.size.h - vertical.0 - vertical.1).max(1);
    rect
}
fn fit_pair(size: i32, first: i32, second: i32) -> (i32, i32) {
    let available = (size - 1).max(0);
    let total = first + second;
    if total <= available {
        (first, second)
    } else if total == 0 {
        (0, 0)
    } else {
        let first = first * available / total;
        (first, available - first)
    }
}
pub(crate) fn client_rect(
    rect: Geometry,
    appearance: &WindowAppearance,
    view: ViewEffects,
    tiled: bool,
) -> Geometry {
    let rect = if view.gaps && tiled {
        inset(rect, appearance.gaps.inner)
    } else {
        rect
    };
    if view.borders {
        inset(rect, appearance.border.width)
    } else {
        rect
    }
}
pub(crate) fn constrain_floating(
    mut rect: Geometry,
    bounds: Geometry,
    appearance: &WindowAppearance,
    view: ViewEffects,
) -> Geometry {
    let bounds = if view.borders {
        inset(bounds, appearance.border.width)
    } else {
        bounds
    };
    rect.size.w = rect.size.w.min(bounds.size.w).max(1);
    rect.size.h = rect.size.h.min(bounds.size.h).max(1);
    rect.loc.x = rect
        .loc
        .x
        .clamp(bounds.loc.x, bounds.loc.x + bounds.size.w - rect.size.w);
    rect.loc.y = rect
        .loc
        .y
        .clamp(bounds.loc.y, bounds.loc.y + bounds.size.h - rect.size.h);
    rect
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn asymmetric_edges_and_tiny_outputs_keep_positive_content() {
        let rect = Rectangle::new((20, 30).into(), (100, 80).into());
        let edges = Edges {
            top: 3,
            right: 5,
            bottom: 7,
            left: 11,
        };
        assert_eq!(
            inset(rect, edges),
            Rectangle::new((31, 33).into(), (84, 70).into())
        );
        for width in 1..20 {
            let tiny = inset(
                Rectangle::from_size((width, 1).into()),
                Edges {
                    top: 4096,
                    right: 4096,
                    bottom: 4096,
                    left: 4096,
                },
            );
            assert_eq!(tiny.size, (1, 1).into());
            assert!(tiny.loc.x < width);
        }
    }
}
pub(crate) mod render;

pub(crate) fn rounded_contains(
    rect: Geometry,
    radii: [f32; 4],
    point: smithay::utils::Point<f64, Logical>,
) -> bool {
    if !rect.to_f64().contains(point) {
        return false;
    }
    let x = (point.x - f64::from(rect.loc.x)) as f32;
    let y = (point.y - f64::from(rect.loc.y)) as f32;
    let w = rect.size.w as f32;
    let h = rect.size.h as f32;
    for (r, cx, cy, in_corner) in [
        (radii[0], radii[0], radii[0], x < radii[0] && y < radii[0]),
        (
            radii[1],
            w - radii[1],
            radii[1],
            x > w - radii[1] && y < radii[1],
        ),
        (
            radii[2],
            w - radii[2],
            h - radii[2],
            x > w - radii[2] && y > h - radii[2],
        ),
        (
            radii[3],
            radii[3],
            h - radii[3],
            x < radii[3] && y > h - radii[3],
        ),
    ] {
        if in_corner && (x - cx).powi(2) + (y - cy).powi(2) > r * r {
            return false;
        }
    }
    true
}
