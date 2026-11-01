//! Player-facing video/input settings, and the rules for nudging them.

/// One adjustable row in the settings screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingId {
    Vsync,
    Sensitivity,
    Fov,
    GuiScale,
    ViewDistance,
}

impl SettingId {
    pub const ALL: [SettingId; 5] = [
        SettingId::Vsync,
        SettingId::Sensitivity,
        SettingId::Fov,
        SettingId::GuiScale,
        SettingId::ViewDistance,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            SettingId::Vsync => "VSync",
            SettingId::Sensitivity => "Mouse Sensitivity",
            SettingId::Fov => "Field of View",
            SettingId::GuiScale => "GUI Scale",
            SettingId::ViewDistance => "Render Distance",
        }
    }

    /// Whether the value can be nudged in this direction.
    pub fn can_adjust(self, settings: &Settings, direction: i32) -> bool {
        let mut probe = *settings;
        probe.adjust(self, direction)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub vsync: bool,
    /// Multiplier on the base mouse sensitivity.
    pub sensitivity: f32,
    pub fov: f32,
    /// 0 = automatic, 1..=4 = fixed.
    pub gui_scale: u8,
    pub view_distance: u8,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            vsync: true,
            sensitivity: 1.0,
            fov: 70.0,
            gui_scale: 0,
            view_distance: 8,
        }
    }
}

impl Settings {
    /// `"ON"` / `"OFF"`, `"70"`, `"8 chunks"`, ...
    pub fn value_label(&self, id: SettingId) -> String {
        match id {
            SettingId::Vsync => if self.vsync { "ON" } else { "OFF" }.to_string(),
            SettingId::Sensitivity => format!("{:.0}%", self.sensitivity * 100.0),
            SettingId::Fov => format!("{:.0}", self.fov),
            SettingId::GuiScale => {
                if self.gui_scale == 0 {
                    "Auto".to_string()
                } else {
                    self.gui_scale.to_string()
                }
            }
            SettingId::ViewDistance => format!("{} chunks", self.view_distance),
        }
    }

    /// Apply a `-1`/`+1` nudge. Returns `true` when the setting changed.
    pub fn adjust(&mut self, id: SettingId, direction: i32) -> bool {
        let before = *self;
        match id {
            SettingId::Vsync => self.vsync = !self.vsync,
            SettingId::Sensitivity => {
                self.sensitivity = (self.sensitivity + 0.1 * direction as f32).clamp(0.4, 3.0);
            }
            SettingId::Fov => {
                self.fov = (self.fov + 5.0 * direction as f32).clamp(50.0, 110.0);
            }
            SettingId::GuiScale => {
                let scale = self.gui_scale as i32 + direction;
                self.gui_scale = scale.rem_euclid(5) as u8;
            }
            SettingId::ViewDistance => {
                let distance = self.view_distance as i32 + direction;
                self.view_distance = distance.clamp(2, 16) as u8;
            }
        }
        *self != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vsync_toggles() {
        let mut settings = Settings::default();
        assert!(settings.vsync);
        assert!(settings.adjust(SettingId::Vsync, 1));
        assert!(!settings.vsync);
        assert_eq!(settings.value_label(SettingId::Vsync), "OFF");
    }

    #[test]
    fn fov_and_sensitivity_clamp() {
        let mut settings = Settings::default();
        for _ in 0..50 {
            settings.adjust(SettingId::Fov, 1);
            settings.adjust(SettingId::Sensitivity, 1);
        }
        assert_eq!(settings.fov, 110.0);
        assert_eq!(settings.sensitivity, 3.0);
        for _ in 0..50 {
            settings.adjust(SettingId::Fov, -1);
            settings.adjust(SettingId::Sensitivity, -1);
        }
        assert_eq!(settings.fov, 50.0);
        assert_eq!(settings.sensitivity, 0.4);
    }

    #[test]
    fn gui_scale_wraps_through_auto() {
        let mut settings = Settings::default();
        assert_eq!(settings.value_label(SettingId::GuiScale), "Auto");
        settings.adjust(SettingId::GuiScale, 1);
        assert_eq!(settings.gui_scale, 1);
        for _ in 0..4 {
            settings.adjust(SettingId::GuiScale, 1);
        }
        assert_eq!(settings.gui_scale, 0, "wraps back to Auto");
    }

    #[test]
    fn render_distance_is_bounded() {
        let mut settings = Settings::default();
        for _ in 0..40 {
            settings.adjust(SettingId::ViewDistance, 1);
        }
        assert_eq!(settings.view_distance, 16);
        assert!(!settings.adjust(SettingId::ViewDistance, 1));
        for _ in 0..40 {
            settings.adjust(SettingId::ViewDistance, -1);
        }
        assert_eq!(settings.view_distance, 2);
        assert!(!settings.adjust(SettingId::ViewDistance, -1));
    }

    #[test]
    fn can_adjust_reports_the_limits() {
        let settings = Settings {
            fov: 110.0,
            ..Settings::default()
        };
        assert!(!SettingId::Fov.can_adjust(&settings, 1));
        assert!(SettingId::Fov.can_adjust(&settings, -1));
    }
}
