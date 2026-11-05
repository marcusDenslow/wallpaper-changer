//! CSS-transition style animation: when a target changes, animate from the current value
//! with a duration, delay and cubic-bezier easing — exactly like the stylesheet did.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ease(pub f32, pub f32, pub f32, pub f32);

pub const MOVE: Ease = Ease(0.22, 0.8, 0.16, 1.0);
pub const OUT: Ease = Ease(0.2, 0.7, 0.2, 1.0);
pub const IN: Ease = Ease(0.5, 0.0, 0.75, 0.0);
pub const FOLD: Ease = Ease(0.55, 0.0, 0.2, 1.0);
/// CSS `ease`.
pub const EASE: Ease = Ease(0.25, 0.1, 0.25, 1.0);
pub const LINEAR: Ease = Ease(0.0, 0.0, 1.0, 1.0);

impl Ease {
    pub fn at(self, x: f32) -> f32 {
        let Ease(x1, y1, x2, y2) = self;
        if x <= 0.0 {
            return 0.0;
        }
        if x >= 1.0 {
            return 1.0;
        }
        let bez = |t: f32, a: f32, b: f32| {
            let u = 1.0 - t;
            3.0 * u * u * t * a + 3.0 * u * t * t * b + t * t * t
        };
        // Solve x(t) = x with Newton, falling back to bisection.
        let mut t = x;
        for _ in 0..8 {
            let err = bez(t, x1, x2) - x;
            if err.abs() < 1e-5 {
                return bez(t, y1, y2);
            }
            let u = 1.0 - t;
            let d = 3.0 * u * u * x1 + 6.0 * u * t * (x2 - x1) + 3.0 * t * t * (1.0 - x2);
            if d.abs() < 1e-6 {
                break;
            }
            t -= err / d;
        }
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        t = x;
        for _ in 0..30 {
            let v = bez(t, x1, x2);
            if (v - x).abs() < 1e-5 {
                break;
            }
            if v < x {
                lo = t;
            } else {
                hi = t;
            }
            t = (lo + hi) / 2.0;
        }
        bez(t, y1, y2)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timing {
    pub dur: f32,
    pub delay: f32,
    pub ease: Ease,
}

pub const fn t(dur: f32, ease: Ease) -> Timing {
    Timing { dur, delay: 0.0, ease }
}

impl Timing {
    pub fn delayed(mut self, ms: f32) -> Self {
        self.delay = ms;
        self
    }
}

/// One animated scalar. Times are in milliseconds.
#[derive(Clone, Copy, Debug)]
pub struct Tween {
    from: f32,
    to: f32,
    start: f64,
    timing: Timing,
}

impl Tween {
    pub fn new(v: f32) -> Self {
        Self { from: v, to: v, start: 0.0, timing: t(0.0, LINEAR) }
    }

    pub fn target(&self) -> f32 {
        self.to
    }

    pub fn get(&self, now: f64) -> f32 {
        let el = (now - self.start) as f32 - self.timing.delay;
        if el <= 0.0 {
            return self.from;
        }
        if self.timing.dur <= 0.0 || el >= self.timing.dur {
            return self.to;
        }
        let k = self.timing.ease.at(el / self.timing.dur);
        self.from + (self.to - self.from) * k
    }

    pub fn active(&self, now: f64) -> bool {
        (now - self.start) as f32 <= self.timing.delay + self.timing.dur && self.from != self.to
    }

    /// Like a CSS property change: animates from the current value to `to`.
    /// Setting the same target again does nothing (no restart).
    pub fn set(&mut self, to: f32, now: f64, timing: Timing) {
        if to == self.to {
            return;
        }
        let cur = self.get(now);
        self.from = cur;
        self.to = to;
        self.start = now;
        self.timing = timing;
    }

    pub fn jump(&mut self, v: f32) {
        self.from = v;
        self.to = v;
        self.timing = t(0.0, LINEAR);
    }
}

/// An RGB colour that transitions (used for the ambient glow).
#[derive(Clone, Copy, Debug)]
pub struct ColorTween(pub [Tween; 3]);

impl ColorTween {
    pub fn new(c: [f32; 3]) -> Self {
        Self([Tween::new(c[0]), Tween::new(c[1]), Tween::new(c[2])])
    }
    pub fn set(&mut self, c: [f32; 3], now: f64, timing: Timing) {
        for i in 0..3 {
            self.0[i].set(c[i], now, timing);
        }
    }
    pub fn get(&self, now: f64) -> [f32; 3] {
        [self.0[0].get(now), self.0[1].get(now), self.0[2].get(now)]
    }
    pub fn active(&self, now: f64) -> bool {
        self.0.iter().any(|t| t.active(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bezier_endpoints_and_monotonic() {
        for e in [MOVE, OUT, IN, FOLD, EASE] {
            assert!(e.at(0.0).abs() < 1e-4);
            assert!((e.at(1.0) - 1.0).abs() < 1e-4);
            let mut last = 0.0;
            for i in 1..=100 {
                let v = e.at(i as f32 / 100.0);
                assert!(v + 1e-3 >= last, "{e:?} not monotonic at {i}");
                last = v;
            }
        }
        // CSS `ease` at 0.5 is ~0.8024
        assert!((EASE.at(0.5) - 0.8024).abs() < 0.01);
    }
    #[test]
    fn tween_retargets_from_current() {
        let mut tw = Tween::new(0.0);
        tw.set(100.0, 0.0, t(100.0, LINEAR));
        assert!((tw.get(50.0) - 50.0).abs() < 0.5);
        tw.set(0.0, 50.0, t(100.0, LINEAR));
        assert!((tw.get(50.0) - 50.0).abs() < 0.5);
        assert!(tw.get(150.0).abs() < 0.01);
    }
}
