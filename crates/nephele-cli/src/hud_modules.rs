//! The HUD modules: what each one shows, whether it is on, and where it sits.
//!
//! Six modules, each independently toggleable and draggable. Positions are
//! stored as a fraction of the window (`0..=1`, top-left of the module) rather
//! than pixels, so a layout arranged at 1280x720 still lands in the same corner
//! at 1920x1080. The persisted form lives in [`crate::settings::Settings`].

use std::collections::VecDeque;
use std::time::Instant;

/// One toggleable overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleId {
    Fps,
    Ping,
    Cps,
    Keystrokes,
    Coords,
    Speed,
}

impl ModuleId {
    pub const ALL: [ModuleId; 6] = [
        ModuleId::Fps,
        ModuleId::Ping,
        ModuleId::Cps,
        ModuleId::Keystrokes,
        ModuleId::Coords,
        ModuleId::Speed,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            ModuleId::Fps => "FPS",
            ModuleId::Ping => "Ping",
            ModuleId::Cps => "CPS",
            ModuleId::Keystrokes => "Keystrokes",
            ModuleId::Coords => "Coordinates",
            ModuleId::Speed => "Speed",
        }
    }

    /// Stable key used in `settings.json`; never change one of these.
    pub const fn key(self) -> &'static str {
        match self {
            ModuleId::Fps => "fps",
            ModuleId::Ping => "ping",
            ModuleId::Cps => "cps",
            ModuleId::Keystrokes => "keystrokes",
            ModuleId::Coords => "coords",
            ModuleId::Speed => "speed",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        ModuleId::ALL.into_iter().find(|id| id.key() == key)
    }

    pub const fn index(self) -> usize {
        match self {
            ModuleId::Fps => 0,
            ModuleId::Ping => 1,
            ModuleId::Cps => 2,
            ModuleId::Keystrokes => 3,
            ModuleId::Coords => 4,
            ModuleId::Speed => 5,
        }
    }

    /// Default top-left, as a fraction of the window.
    ///
    /// Everything that reports a number stacks in one left-hand column; the
    /// keystrokes box sits beside the scoreboard, which is vertically centred
    /// against the right edge. The 0.062 vertical step fits the tallest GUI
    /// scale a 720p window picks (3), where a module is 39 px tall.
    pub const fn default_anchor(self) -> [f32; 2] {
        match self {
            ModuleId::Fps => [0.008, 0.012],
            ModuleId::Ping => [0.008, 0.074],
            ModuleId::Cps => [0.008, 0.136],
            ModuleId::Coords => [0.008, 0.198],
            ModuleId::Speed => [0.008, 0.260],
            ModuleId::Keystrokes => [0.480, 0.020],
        }
    }

    /// Every module starts on; a client that ships hidden overlays is a client
    /// nobody finds the settings for.
    pub const fn default_enabled(self) -> bool {
        true
    }
}

/// One module's saved state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModuleEntry {
    pub enabled: bool,
    pub anchor: [f32; 2],
}

impl ModuleEntry {
    pub fn clamp_anchor(anchor: [f32; 2]) -> [f32; 2] {
        // Keep the top-left inside the window; the renderer clamps the right
        // and bottom edges once it knows how wide the module ended up.
        [anchor[0].clamp(0.0, 1.0), anchor[1].clamp(0.0, 1.0)]
    }
}

/// All six module states.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModuleConfig {
    entries: [ModuleEntry; 6],
}

impl Default for ModuleConfig {
    fn default() -> Self {
        Self {
            entries: ModuleId::ALL.map(|id| ModuleEntry {
                enabled: id.default_enabled(),
                anchor: id.default_anchor(),
            }),
        }
    }
}

impl ModuleConfig {
    pub fn get(&self, id: ModuleId) -> ModuleEntry {
        self.entries[id.index()]
    }

    pub fn get_mut(&mut self, id: ModuleId) -> &mut ModuleEntry {
        &mut self.entries[id.index()]
    }

    pub fn is_enabled(&self, id: ModuleId) -> bool {
        self.get(id).enabled
    }

    /// Flip one module. Returns the new state.
    pub fn toggle(&mut self, id: ModuleId) -> bool {
        let entry = self.get_mut(id);
        entry.enabled = !entry.enabled;
        entry.enabled
    }

    pub fn set_anchor(&mut self, id: ModuleId, anchor: [f32; 2]) {
        self.get_mut(id).anchor = ModuleEntry::clamp_anchor(anchor);
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Which key a keystroke box draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyBox {
    Forward,
    Left,
    Back,
    Right,
    Jump,
    Sneak,
    Attack,
    UseItem,
}

/// Pressed state of everything the keystrokes module and the CPS module show.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KeyState {
    pub forward: bool,
    pub left: bool,
    pub back: bool,
    pub right: bool,
    pub jump: bool,
    pub sneak: bool,
    pub attack: bool,
    pub use_item: bool,
}

impl KeyState {
    pub fn pressed(&self, key: KeyBox) -> bool {
        match key {
            KeyBox::Forward => self.forward,
            KeyBox::Left => self.left,
            KeyBox::Back => self.back,
            KeyBox::Right => self.right,
            KeyBox::Jump => self.jump,
            KeyBox::Sneak => self.sneak,
            KeyBox::Attack => self.attack,
            KeyBox::UseItem => self.use_item,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseSide {
    Left,
    Right,
}

/// Clicks in the last second, one window per mouse button.
#[derive(Debug, Clone, Default)]
pub struct ClickCounter {
    left: VecDeque<Instant>,
    right: VecDeque<Instant>,
}

impl ClickCounter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn click(&mut self, side: MouseSide, now: Instant) {
        let queue = match side {
            MouseSide::Left => &mut self.left,
            MouseSide::Right => &mut self.right,
        };
        queue.push_back(now);
    }

    /// Clicks per second for `(left, right)`, pruning anything older than 1 s.
    pub fn counts(&mut self, now: Instant) -> (u32, u32) {
        let cutoff = now.checked_sub(std::time::Duration::from_secs(1));
        let prune = |queue: &mut VecDeque<Instant>| {
            if let Some(cutoff) = cutoff {
                while queue.front().is_some_and(|at| *at <= cutoff) {
                    queue.pop_front();
                }
            }
            queue.len() as u32
        };
        (prune(&mut self.left), prune(&mut self.right))
    }
}

/// Horizontal speed in blocks per second, smoothed over a short window.
#[derive(Debug, Clone, Default)]
pub struct Speedometer {
    last: Option<(Instant, [f64; 3])>,
    pub blocks_per_second: f32,
}

impl Speedometer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one position sample. Samples closer together than
    /// [`Speedometer::MIN_INTERVAL`] are ignored, so a 2000 fps burst cannot
    /// divide by a near-zero delta.
    pub fn sample(&mut self, at: Instant, position: [f64; 3]) -> f32 {
        let Some((last_at, last)) = self.last else {
            self.last = Some((at, position));
            return 0.0;
        };
        let dt = at.saturating_duration_since(last_at).as_secs_f32();
        if dt < 1.0 / 240.0 {
            return self.blocks_per_second;
        }
        self.last = Some((at, position));
        let dx = position[0] - last[0];
        let dz = position[2] - last[2];
        let instant = (dx * dx + dz * dz).sqrt() as f32 / dt;
        // Exponential smoothing with a ~0.2 s time constant.
        let alpha = 1.0 - (-dt / 0.2).exp();
        self.blocks_per_second += (instant - self.blocks_per_second) * alpha;
        self.blocks_per_second
    }
}

/// Everything the module renderer needs from the game this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Values {
    pub fps: f32,
    /// `None` until the server's tab list has told us our own latency.
    pub ping: Option<i32>,
    pub cps: (u32, u32),
    pub position: [f64; 3],
    pub speed: f32,
    pub keys: KeyState,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn every_module_has_a_distinct_stable_key_and_anchor() {
        let mut keys: Vec<&str> = ModuleId::ALL.iter().map(|id| id.key()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), ModuleId::ALL.len());

        for id in ModuleId::ALL {
            let anchor = id.default_anchor();
            assert!((0.0..1.0).contains(&anchor[0]), "{id:?} x");
            assert!((0.0..1.0).contains(&anchor[1]), "{id:?} y");
            assert_eq!(ModuleId::from_key(id.key()), Some(id));
        }

        // Everything but the keystrokes box stacks in one left-hand column.
        let column_x = ModuleId::Fps.default_anchor()[0];
        let mut ys = Vec::new();
        for id in ModuleId::ALL {
            let anchor = id.default_anchor();
            if id == ModuleId::Keystrokes {
                assert_ne!(anchor[0], column_x, "keystrokes sits on its own");
                continue;
            }
            assert_eq!(anchor[0], column_x, "{id:?} left the column");
            ys.push(anchor[1]);
        }
        ys.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let unique = ys.len();
        ys.dedup();
        assert_eq!(ys.len(), unique, "the column stacks two modules");
    }

    #[test]
    fn toggle_flips_one_module_and_leaves_the_rest_alone() {
        let mut config = ModuleConfig::default();
        assert!(config.is_enabled(ModuleId::Fps));
        assert!(!config.toggle(ModuleId::Fps));
        assert!(!config.is_enabled(ModuleId::Fps));
        assert!(config.is_enabled(ModuleId::Ping));
    }

    #[test]
    fn anchors_are_clamped_to_the_window() {
        let mut config = ModuleConfig::default();
        config.set_anchor(ModuleId::Fps, [-4.0, 9.0]);
        assert_eq!(config.get(ModuleId::Fps).anchor, [0.0, 1.0]);
    }

    #[test]
    fn reset_restores_defaults() {
        let mut config = ModuleConfig::default();
        config.toggle(ModuleId::Cps);
        config.set_anchor(ModuleId::Cps, [0.5, 0.5]);
        config.reset();
        assert_eq!(config, ModuleConfig::default());
    }

    #[test]
    fn clicks_older_than_a_second_stop_counting() {
        let mut counter = ClickCounter::new();
        let start = Instant::now();
        for _ in 0..6 {
            counter.click(MouseSide::Left, start);
        }
        counter.click(MouseSide::Right, start);
        assert_eq!(counter.counts(start), (6, 1));

        let later = start + Duration::from_millis(1500);
        assert_eq!(counter.counts(later), (0, 0), "the window slid past them");

        counter.click(MouseSide::Left, later);
        assert_eq!(counter.counts(later), (1, 0));
    }

    #[test]
    fn speed_is_horizontal_only_and_ignores_tiny_deltas() {
        let mut meter = Speedometer::new();
        let start = Instant::now();
        meter.sample(start, [0.0, 64.0, 0.0]);
        // One block east per tick: 4.317 blocks/second at 20 Hz.
        let mut at = start;
        for step in 1..=40 {
            at = start + Duration::from_millis(50 * step);
            meter.sample(at, [step as f64 * 0.1, 64.0 + step as f64, 0.0]);
        }
        assert!(
            (meter.blocks_per_second - 2.0).abs() < 0.2,
            "0.1 blocks per tick is 2 blocks/second: {}",
            meter.blocks_per_second
        );

        let before = meter.blocks_per_second;
        meter.sample(at, [99.0, 70.0, 99.0]);
        assert_eq!(
            meter.blocks_per_second, before,
            "a sub-4 ms delta is not a sample"
        );
    }

    #[test]
    fn key_state_reports_the_pressed_box() {
        let keys = KeyState {
            forward: true,
            ..KeyState::default()
        };
        assert!(keys.pressed(KeyBox::Forward));
        assert!(!keys.pressed(KeyBox::Back));
    }
}
