use std::f32::consts::PI;

use super::oscillator::Waveform;

pub const MIN_LFO_HZ: f32 = 0.01;
pub const MAX_LFO_HZ: f32 = 30.0;

/// Most nodes an LFO shape can hold. Fixed so shapes are `Copy` and never
/// allocate on the audio thread.
pub const MAX_LFO_POINTS: usize = 32;
/// Strongest bend of a segment, in either direction.
pub const MAX_LFO_CURVE: f32 = 12.0;

/// One node of an LFO shape. `curve` bends the segment that leaves this node:
/// 0 is straight, positive values ease in (slow start), negative ease out.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LfoPoint {
    /// Position in the cycle, `0.0..=1.0`.
    pub x: f32,
    /// Output value, `-1.0..=1.0`.
    pub y: f32,
    pub curve: f32,
}

impl LfoPoint {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y, curve: 0.0 }
    }

    pub const fn curved(x: f32, y: f32, curve: f32) -> Self {
        Self { x, y, curve }
    }

    fn sanitized(self) -> Self {
        let finite = |value: f32| if value.is_finite() { value } else { 0.0 };
        Self {
            x: finite(self.x).clamp(0.0, 1.0),
            y: finite(self.y).clamp(-1.0, 1.0),
            curve: finite(self.curve).clamp(-MAX_LFO_CURVE, MAX_LFO_CURVE),
        }
    }
}

/// A periodic, user-editable LFO shape: nodes sorted by `x`, joined by
/// (optionally bent) segments, with the last node joining the first one in
/// the next cycle. Two nodes sharing an `x` make a vertical jump.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LfoShape {
    points: [LfoPoint; MAX_LFO_POINTS],
    len: usize,
    /// Eases every segment with a half cosine, rounding the corners.
    smooth: bool,
}

impl Default for LfoShape {
    fn default() -> Self {
        Self::from_waveform(Waveform::Sine)
    }
}

impl LfoShape {
    /// Builds a shape from `points` (sorted by `x`, clamped, and truncated to
    /// [`MAX_LFO_POINTS`]). An empty list gives a flat line at 0.
    pub fn new(points: &[LfoPoint], smooth: bool) -> Self {
        let mut shape = Self {
            points: [LfoPoint::default(); MAX_LFO_POINTS],
            len: 0,
            smooth,
        };
        for point in points.iter().take(MAX_LFO_POINTS) {
            shape.points[shape.len] = point.sanitized();
            shape.len += 1;
        }
        if shape.len == 0 {
            shape.points[0] = LfoPoint::new(0.0, 0.0);
            shape.len = 1;
        }
        // Stable, so nodes sharing an `x` keep their order (and their jump).
        shape.points[..shape.len].sort_by(|a, b| a.x.total_cmp(&b.x));
        shape
    }

    /// The node layout that reproduces `waveform` exactly.
    pub fn from_waveform(waveform: Waveform) -> Self {
        match waveform {
            Waveform::Sine => {
                Self::new(&[LfoPoint::new(0.25, 1.0), LfoPoint::new(0.75, -1.0)], true)
            }
            Waveform::Triangle => Self::new(
                &[LfoPoint::new(0.25, 1.0), LfoPoint::new(0.75, -1.0)],
                false,
            ),
            Waveform::Square => Self::new(
                &[
                    LfoPoint::new(0.0, 1.0),
                    LfoPoint::new(0.5, 1.0),
                    LfoPoint::new(0.5, -1.0),
                    LfoPoint::new(1.0, -1.0),
                ],
                false,
            ),
            Waveform::Sawtooth => {
                Self::new(&[LfoPoint::new(0.5, 1.0), LfoPoint::new(0.5, -1.0)], false)
            }
        }
    }

    pub fn points(&self) -> &[LfoPoint] {
        &self.points[..self.len]
    }

    pub fn is_smooth(&self) -> bool {
        self.smooth
    }

    pub fn set_smooth(&mut self, smooth: bool) {
        self.smooth = smooth;
    }

    /// The shape's value at normalized `phase` (wrapped into `0.0..1.0`).
    pub fn sample(&self, phase: f32) -> f32 {
        let phase = if phase.is_finite() {
            phase.rem_euclid(1.0)
        } else {
            0.0
        };
        let points = self.points();
        let last = points.len() - 1;
        let (index, x) = if phase < points[0].x {
            (last, phase + 1.0)
        } else {
            // The last node at or before `phase`, so at a jump the later
            // node wins.
            (points.partition_point(|point| point.x <= phase) - 1, phase)
        };
        let (end_y, end_x) = self.segment_end(index);
        segment_value(points[index], end_y, end_x, x, self.smooth)
    }

    /// Where the segment leaving node `index` ends; the last segment ends at
    /// the first node of the next cycle.
    fn segment_end(&self, index: usize) -> (f32, f32) {
        match self.points().get(index + 1) {
            Some(next) => (next.y, next.x),
            None => (self.points[0].y, self.points[0].x + 1.0),
        }
    }

    /// The value at the middle of the segment leaving node `index`, where the
    /// editor draws that segment's bend handle.
    pub fn segment_midpoint(&self, index: usize) -> Option<(f32, f32)> {
        let points = self.points();
        let start = *points.get(index)?;
        let (end_y, end_x) = self.segment_end(index);
        let x = (start.x + end_x) * 0.5;
        let value = segment_value(start, end_y, end_x, x, self.smooth);
        Some((if x >= 1.0 { x - 1.0 } else { x }, value))
    }

    /// Adds a node at (`x`, `y`) and returns its index, or `None` when full.
    pub fn insert_point(&mut self, x: f32, y: f32) -> Option<usize> {
        if self.len >= MAX_LFO_POINTS {
            return None;
        }
        let point = LfoPoint::new(x, y).sanitized();
        let index = self.points().partition_point(|p| p.x <= point.x);
        self.points.copy_within(index..self.len, index + 1);
        self.points[index] = point;
        self.len += 1;
        Some(index)
    }

    /// Removes node `index`, keeping at least one node. Returns whether it was
    /// removed.
    pub fn remove_point(&mut self, index: usize) -> bool {
        if index >= self.len || self.len <= 1 {
            return false;
        }
        self.points.copy_within(index + 1..self.len, index);
        self.len -= 1;
        true
    }

    /// Moves node `index`, keeping it between its neighbours so the order (and
    /// the segments) stay the same.
    pub fn move_point(&mut self, index: usize, x: f32, y: f32) {
        if index >= self.len {
            return;
        }
        let low = if index == 0 {
            0.0
        } else {
            self.points[index - 1].x
        };
        let high = if index + 1 >= self.len {
            1.0
        } else {
            self.points[index + 1].x
        };
        let moved = LfoPoint {
            x,
            y,
            ..self.points[index]
        }
        .sanitized();
        self.points[index].x = moved.x.clamp(low, high);
        self.points[index].y = moved.y;
    }

    /// Sets the bend of the segment leaving node `index`.
    pub fn set_curve(&mut self, index: usize, curve: f32) {
        if index < self.len {
            self.points[index].curve = LfoPoint {
                curve,
                ..self.points[index]
            }
            .sanitized()
            .curve;
        }
    }
}

/// Value on the segment from `start` to (`end_x`, `end_y`) at `x`.
fn segment_value(start: LfoPoint, end_y: f32, end_x: f32, x: f32, smooth: bool) -> f32 {
    let width = end_x - start.x;
    if width <= 0.0 {
        return start.y;
    }
    let mut t = ((x - start.x) / width).clamp(0.0, 1.0);
    t = bend(t, start.curve);
    if smooth {
        t = 0.5 - 0.5 * (PI * t).cos();
    }
    start.y + (end_y - start.y) * t
}

/// Exponential bend of `t` in `0..=1`; 0 leaves it linear.
fn bend(t: f32, curve: f32) -> f32 {
    if curve.abs() < 1e-4 {
        t
    } else {
        (curve * t).exp_m1() / curve.exp_m1()
    }
}

/// How LFOs relate to notes. The engine applies the mode; an `Lfo` itself
/// only runs between `start` and `stop`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LfoMode {
    /// Each note gets its own LFO, started from phase 0 when the note is
    /// pressed and running until the note finishes sounding.
    #[default]
    Trigger,
    /// One LFO runs continuously, shared by every note.
    Sync,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LfoSettings {
    pub shape: LfoShape,
    pub frequency_hz: f32,
    pub mode: LfoMode,
}

impl Default for LfoSettings {
    fn default() -> Self {
        Self {
            shape: LfoShape::default(),
            frequency_hz: 1.0,
            mode: LfoMode::Trigger,
        }
    }
}

/// A low-frequency oscillator producing a bipolar (-1 to 1) control signal.
/// The shape is node based and evaluated without band-limiting.
#[derive(Debug)]
pub struct Lfo {
    sample_rate: f64,
    /// Normalized phase in `0.0..1.0`. Kept in f64 because at 0.01 Hz and
    /// high sample rates the per-sample increment is below f32 resolution.
    phase: f64,
    shape: LfoShape,
    frequency_hz: f32,
    running: bool,
}

impl Default for Lfo {
    fn default() -> Self {
        let settings = LfoSettings::default();
        Self {
            sample_rate: 44_100.0,
            phase: 0.0,
            shape: settings.shape,
            frequency_hz: settings.frequency_hz,
            running: false,
        }
    }
}

impl Lfo {
    /// Stops the LFO and rewinds it.
    pub fn reset(&mut self, sample_rate: f32) {
        self.sample_rate = f64::from(sample_rate);
        self.phase = 0.0;
        self.running = false;
    }

    /// Applies shape and rate; the mode is handled by the engine.
    pub fn set_settings(&mut self, settings: LfoSettings) {
        self.shape = settings.shape;
        self.frequency_hz = if settings.frequency_hz.is_finite() {
            settings.frequency_hz.clamp(MIN_LFO_HZ, MAX_LFO_HZ)
        } else {
            MIN_LFO_HZ
        };
    }

    /// Starts (or restarts) the cycle from phase 0.
    pub fn start(&mut self) {
        self.phase = 0.0;
        self.running = true;
    }

    pub fn stop(&mut self) {
        self.running = false;
    }

    /// The current value, then advances one sample. Returns 0 while stopped.
    pub fn next_sample(&mut self) -> f32 {
        if !self.running {
            return 0.0;
        }
        let value = self.shape.sample(self.phase as f32);
        let increment = f64::from(self.frequency_hz) / self.sample_rate;
        self.phase = (self.phase + increment).fract();
        value
    }

    /// Normalized position in the cycle, `0.0..1.0`.
    pub fn phase(&self) -> f32 {
        self.phase as f32
    }

    pub fn is_running(&self) -> bool {
        self.running
    }
}

/// Fills `samples` with one LFO cycle of `shape`, 0 to 360 degrees
/// inclusive, using the same function the LFO runs. Intended for displays.
pub fn render_lfo_cycle(shape: &LfoShape, samples: &mut [f32]) {
    let Some(segments) = samples.len().checked_sub(1).filter(|&n| n > 0) else {
        samples.fill(0.0);
        return;
    };
    for (index, sample) in samples.iter_mut().enumerate() {
        let phase = (index as f32 / segments as f32).fract();
        *sample = shape.sample(phase);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running_lfo(waveform: Waveform, frequency_hz: f32, sample_rate: f32) -> Lfo {
        let mut lfo = Lfo::default();
        lfo.reset(sample_rate);
        lfo.set_settings(LfoSettings {
            shape: LfoShape::from_waveform(waveform),
            frequency_hz,
            mode: LfoMode::Trigger,
        });
        lfo.start();
        lfo
    }

    #[test]
    fn completes_one_cycle_per_period() {
        let mut lfo = running_lfo(Waveform::Sine, 2.0, 1_000.0);
        let samples: Vec<f32> = (0..500).map(|_| lfo.next_sample()).collect();
        assert!(samples[0].abs() < 1e-6);
        assert!((samples[125] - 1.0).abs() < 1e-4);
        assert!((samples[375] + 1.0).abs() < 1e-4);
        assert!(lfo.phase() < 1e-6 || lfo.phase() > 1.0 - 1e-6);
    }

    #[test]
    fn shapes_are_unfiltered() {
        let mut square = running_lfo(Waveform::Square, 1.0, 100.0);
        let values: Vec<f32> = (0..100).map(|_| square.next_sample()).collect();
        assert!(values[1..50].iter().all(|&v| v == 1.0));
        assert!(values[50..].iter().all(|&v| v == -1.0));

        let mut saw = running_lfo(Waveform::Sawtooth, 1.0, 100.0);
        let values: Vec<f32> = (0..100).map(|_| saw.next_sample()).collect();
        assert!((values[49] - 0.98).abs() < 1e-4);
        assert!((values[50] + 1.0).abs() < 1e-4);
    }

    #[test]
    fn rate_is_clamped_to_its_range() {
        for (input, expected) in [
            (0.0, MIN_LFO_HZ),
            (100.0, MAX_LFO_HZ),
            (f32::NAN, MIN_LFO_HZ),
            (5.0, 5.0),
        ] {
            let lfo = running_lfo(Waveform::Sine, input, 48_000.0);
            assert_eq!(lfo.frequency_hz, expected, "{input}");
        }
    }

    #[test]
    fn slowest_rate_still_advances_at_high_sample_rates() {
        let mut lfo = running_lfo(Waveform::Sine, MIN_LFO_HZ, 192_000.0);
        for _ in 0..192_000 {
            lfo.next_sample();
        }
        assert!((lfo.phase() - 0.01).abs() < 1e-6, "{}", lfo.phase());
    }

    #[test]
    fn runs_only_between_start_and_stop() {
        let mut lfo = Lfo::default();
        lfo.reset(1_000.0);
        lfo.set_settings(LfoSettings {
            frequency_hz: 10.0,
            ..LfoSettings::default()
        });
        assert!(!lfo.is_running());
        assert_eq!(lfo.next_sample(), 0.0);
        assert_eq!(lfo.phase(), 0.0);

        lfo.start();
        for _ in 0..30 {
            lfo.next_sample();
        }
        assert!((lfo.phase() - 0.3).abs() < 1e-6);

        lfo.start();
        assert_eq!(lfo.phase(), 0.0);

        lfo.stop();
        assert!(!lfo.is_running());
        lfo.next_sample();
        assert_eq!(lfo.phase(), 0.0);
    }

    #[test]
    fn rendered_cycle_matches_the_lfo() {
        for waveform in [
            Waveform::Sine,
            Waveform::Square,
            Waveform::Triangle,
            Waveform::Sawtooth,
        ] {
            let mut samples = [0.0; 65];
            render_lfo_cycle(&LfoShape::from_waveform(waveform), &mut samples);
            let mut lfo = running_lfo(waveform, 1.0, 64.0);
            for (index, expected) in samples[..64].iter().enumerate() {
                let actual = lfo.next_sample();
                assert!((actual - expected).abs() < 1e-5, "{waveform:?} {index}");
            }
        }
    }

    #[test]
    fn presets_match_the_oscillator_waveforms() {
        use super::super::oscillator::naive_waveform_sample;
        for waveform in [
            Waveform::Sine,
            Waveform::Square,
            Waveform::Triangle,
            Waveform::Sawtooth,
        ] {
            let shape = LfoShape::from_waveform(waveform);
            for step in 0..1000 {
                let phase = step as f32 / 1000.0;
                let expected = naive_waveform_sample(waveform, phase);
                let actual = shape.sample(phase);
                assert!(
                    (actual - expected).abs() < 1e-4,
                    "{waveform:?} {phase}: {actual} vs {expected}"
                );
            }
        }
    }

    #[test]
    fn last_node_wraps_to_the_first() {
        let shape = LfoShape::new(&[LfoPoint::new(0.2, 1.0), LfoPoint::new(0.6, -1.0)], false);
        assert!((shape.sample(0.2) - 1.0).abs() < 1e-6);
        assert!((shape.sample(0.4) - 0.0).abs() < 1e-6);
        // 0.6 -> 1.2 runs from -1 to 1; 0.0 (1.0) is two thirds of the way.
        assert!((shape.sample(0.0) - 1.0 / 3.0).abs() < 1e-5);
        assert!((shape.sample(1.0) - shape.sample(0.0)).abs() < 1e-6);
        let single = LfoShape::new(&[LfoPoint::new(0.3, 0.5)], false);
        assert_eq!(single.sample(0.1), 0.5);
        assert_eq!(single.sample(0.9), 0.5);
    }

    #[test]
    fn curves_bend_segments_both_ways() {
        let up = |curve| {
            LfoShape::new(
                &[LfoPoint::curved(0.0, -1.0, curve), LfoPoint::new(1.0, 1.0)],
                false,
            )
        };
        assert!((up(0.0).sample(0.5)).abs() < 1e-6);
        assert!(up(4.0).sample(0.5) < -0.5);
        assert!(up(-4.0).sample(0.5) > 0.5);
        assert_eq!(up(4.0).sample(0.0), -1.0);
    }

    #[test]
    fn editing_keeps_nodes_ordered_and_bounded() {
        let mut shape = LfoShape::from_waveform(Waveform::Triangle);
        assert_eq!(shape.insert_point(0.5, 2.0), Some(1));
        assert_eq!(shape.points()[1], LfoPoint::new(0.5, 1.0));
        shape.move_point(1, 0.9, -0.5);
        assert_eq!(shape.points()[1], LfoPoint::new(0.75, -0.5));
        shape.move_point(0, -1.0, 0.0);
        assert_eq!(shape.points()[0].x, 0.0);
        assert!(shape.remove_point(1));
        assert!(shape.remove_point(0));
        assert!(!shape.remove_point(0));
        assert_eq!(shape.points().len(), 1);
        for _ in 0..MAX_LFO_POINTS {
            shape.insert_point(0.5, 0.0);
        }
        assert_eq!(shape.points().len(), MAX_LFO_POINTS);
        assert_eq!(shape.insert_point(0.5, 0.0), None);
        shape.set_curve(0, 100.0);
        assert_eq!(shape.points()[0].curve, MAX_LFO_CURVE);
    }

    #[test]
    fn segment_midpoints_follow_the_shape() {
        let shape = LfoShape::from_waveform(Waveform::Triangle);
        assert_eq!(shape.segment_midpoint(0), Some((0.5, 0.0)));
        let (x, y) = shape.segment_midpoint(1).unwrap();
        assert!((x - 0.0).abs() < 1e-6 && y.abs() < 1e-6, "{x} {y}");
        assert_eq!(shape.segment_midpoint(2), None);
    }
}
