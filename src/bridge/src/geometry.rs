//! Point-to-segment distance calculations used when drawing the icons and the canvas image.

pub type Point = (f32, f32);

/// The position t (0 at a, 1 at b) of the point on segment a–b closest to p, and the distance to that point
pub fn closest_on_segment(p: Point, a: Point, b: Point) -> (f32, f32) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length_squared = dx * dx + dy * dy;
    let t = if length_squared == 0.0 {
        0.0
    } else {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / length_squared).clamp(0.0, 1.0)
    };
    (t, (p.0 - (a.0 + dx * t)).hypot(p.1 - (a.1 + dy * t)))
}

/// Distance to a polyline
#[cfg(any(test, target_os = "macos", target_os = "windows"))]
pub fn polyline_distance(p: Point, points: &[Point]) -> f32 {
    points
        .windows(2)
        .map(|w| closest_on_segment(p, w[0], w[1]).1)
        .fold(f32::INFINITY, f32::min)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closest_point_is_clamped_to_segment() {
        assert_eq!(
            closest_on_segment((5.0, 3.0), (0.0, 0.0), (10.0, 0.0)),
            (0.5, 3.0)
        );
        assert_eq!(
            closest_on_segment((-4.0, 3.0), (0.0, 0.0), (10.0, 0.0)),
            (0.0, 5.0)
        );
        assert_eq!(
            closest_on_segment((3.0, 4.0), (0.0, 0.0), (0.0, 0.0)),
            (0.0, 5.0)
        );
    }

    #[test]
    fn polyline_distance_uses_nearest_segment() {
        let line = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)];
        assert_eq!(polyline_distance((12.0, 5.0), &line), 2.0);
    }
}
