#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimelineViewport {
    left: f32,
    top: f32,
    width: f32,
    height: f32,
}

impl Default for TimelineViewport {
    fn default() -> Self {
        Self::new(0.0, 0.0, 900.0, 520.0)
    }
}

impl TimelineViewport {
    pub fn new(left: f32, top: f32, width: f32, height: f32) -> Self {
        Self { left: left.max(0.0), top: top.max(0.0), width: width.max(1.0), height: height.max(1.0) }
    }

    pub fn contains(self, start: f32, width: f32, lane: i32) -> bool {
        const LANE: f32 = 84.0;
        const EDGE: f32 = 10.0;
        let x = start + EDGE;
        let y = lane as f32 * LANE;
        // One screen on either side gives pointer gestures and wheel scroll room
        // without mounting all the clip handles in a long project.
        x + width.max(1.0) >= self.left - self.width
            && x <= self.left + 2.0 * self.width
            && y + LANE >= self.top - LANE
            && y <= self.top + self.height + LANE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_keeps_long_intersecting_rows_and_overscan_but_drops_remote_clips() {
        let viewport = TimelineViewport::new(1_000.0, 168.0, 200.0, 84.0);
        assert!(viewport.contains(200.0, 1_000.0, 2));
        assert!(viewport.contains(1_300.0, 20.0, 3));
        assert!(viewport.contains(1_200.0, 20.0, 1));
        assert!(!viewport.contains(300.0, 20.0, 2));
        assert!(!viewport.contains(1_700.0, 20.0, 2));
        assert!(!viewport.contains(1_001.0, 20.0, 8));
    }
}
