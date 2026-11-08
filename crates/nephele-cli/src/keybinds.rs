//! Rebinding: the actions the client implements, their default keys, and the
//! table that turns a binding into a display name and back.
//!
//! Bindings are persisted as text (`"key:KeyW"`, `"mouse:Left"`, `"none"`) so
//! `settings.json` stays readable and a key this table does not know degrades to
//! `NONE` instead of failing the whole file.

use winit::event::MouseButton;
use winit::keyboard::KeyCode;

/// What a key or mouse button is bound to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bind {
    Key(KeyCode),
    Mouse(MouseButton),
    Unbound,
}

impl Bind {
    /// `"key:KeyW"`, `"mouse:Left"`, `"none"`.
    pub fn encode(self) -> String {
        match self {
            // Display names are the stable spelling: `key:W`, `key:LSHIFT`.
            Bind::Key(code) => format!("key:{}", key_name(code)),
            Bind::Mouse(button) => format!("mouse:{button:?}"),
            Bind::Unbound => "none".to_string(),
        }
    }

    /// Inverse of [`Bind::encode`]; an unknown key is `Unbound`, not an error.
    pub fn decode(text: &str) -> Self {
        if text == "none" {
            return Bind::Unbound;
        }
        if let Some(name) = text.strip_prefix("mouse:") {
            return match name {
                "Left" => Bind::Mouse(MouseButton::Left),
                "Right" => Bind::Mouse(MouseButton::Right),
                "Middle" => Bind::Mouse(MouseButton::Middle),
                "Back" => Bind::Mouse(MouseButton::Back),
                "Forward" => Bind::Mouse(MouseButton::Forward),
                _ => Bind::Unbound,
            };
        }
        let Some(name) = text.strip_prefix("key:") else {
            return Bind::Unbound;
        };
        KEY_NAMES
            .iter()
            .find(|(_, display)| *display == name)
            .map(|(code, _)| Bind::Key(*code))
            .unwrap_or(Bind::Unbound)
    }

    /// How the controls screen labels this binding.
    pub fn name(self) -> &'static str {
        match self {
            Bind::Key(code) => key_name(code),
            Bind::Mouse(MouseButton::Left) => "Button 1",
            Bind::Mouse(MouseButton::Right) => "Button 2",
            Bind::Mouse(MouseButton::Middle) => "Button 3",
            Bind::Mouse(MouseButton::Back) => "Button 4",
            Bind::Mouse(MouseButton::Forward) => "Button 5",
            Bind::Mouse(_) | Bind::Unbound => "NONE",
        }
    }
}

/// One binding row. Order here is the order the controls screen shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Forward,
    Back,
    Left,
    Right,
    Jump,
    Sneak,
    Sprint,
    /// Latched rather than held: pressing flips the state.
    ToggleSprint,
    ToggleSneak,
    Attack,
    UseItem,
    PickBlock,
    DropItem,
    Hotbar(u8),
    Inventory,
    Chat,
    Command,
    Fullscreen,
}

/// Grouping in the controls screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Movement,
    Gameplay,
    Inventory,
    Multiplayer,
    Miscellaneous,
}

impl Category {
    pub const ALL: [Category; 5] = [
        Category::Movement,
        Category::Gameplay,
        Category::Inventory,
        Category::Multiplayer,
        Category::Miscellaneous,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Category::Movement => "Movement",
            Category::Gameplay => "Gameplay",
            Category::Inventory => "Inventory",
            Category::Multiplayer => "Multiplayer",
            Category::Miscellaneous => "Miscellaneous",
        }
    }
}

impl Action {
    /// Every row, in display order.
    pub const ALL: [Action; 26] = [
        Action::Forward,
        Action::Back,
        Action::Left,
        Action::Right,
        Action::Jump,
        Action::Sneak,
        Action::Sprint,
        Action::ToggleSprint,
        Action::ToggleSneak,
        Action::Attack,
        Action::UseItem,
        Action::PickBlock,
        Action::DropItem,
        Action::Hotbar(1),
        Action::Hotbar(2),
        Action::Hotbar(3),
        Action::Hotbar(4),
        Action::Hotbar(5),
        Action::Hotbar(6),
        Action::Hotbar(7),
        Action::Hotbar(8),
        Action::Hotbar(9),
        Action::Inventory,
        Action::Chat,
        Action::Command,
        Action::Fullscreen,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Action::Forward => "Walk Forward",
            Action::Back => "Walk Backwards",
            Action::Left => "Strafe Left",
            Action::Right => "Strafe Right",
            Action::Jump => "Jump",
            Action::Sneak => "Sneak",
            Action::Sprint => "Sprint",
            Action::ToggleSprint => "Toggle Sprint",
            Action::ToggleSneak => "Toggle Sneak",
            Action::Attack => "Attack/Destroy",
            Action::UseItem => "Use Item/Place Block",
            Action::PickBlock => "Pick Block",
            Action::DropItem => "Drop Item",
            Action::Hotbar(1) => "Hotbar Slot 1",
            Action::Hotbar(2) => "Hotbar Slot 2",
            Action::Hotbar(3) => "Hotbar Slot 3",
            Action::Hotbar(4) => "Hotbar Slot 4",
            Action::Hotbar(5) => "Hotbar Slot 5",
            Action::Hotbar(6) => "Hotbar Slot 6",
            Action::Hotbar(7) => "Hotbar Slot 7",
            Action::Hotbar(8) => "Hotbar Slot 8",
            Action::Hotbar(9) => "Hotbar Slot 9",
            Action::Hotbar(_) => "Hotbar Slot",
            Action::Inventory => "Inventory",
            Action::Chat => "Open Chat",
            Action::Command => "Open Command",
            Action::Fullscreen => "Toggle Fullscreen",
        }
    }

    pub fn category(self) -> Category {
        match self {
            Action::Forward
            | Action::Back
            | Action::Left
            | Action::Right
            | Action::Jump
            | Action::Sneak
            | Action::Sprint
            | Action::ToggleSprint
            | Action::ToggleSneak => Category::Movement,
            Action::Attack | Action::UseItem | Action::PickBlock | Action::DropItem => {
                Category::Gameplay
            }
            Action::Hotbar(_) | Action::Inventory => Category::Inventory,
            Action::Chat | Action::Command => Category::Multiplayer,
            Action::Fullscreen => Category::Miscellaneous,
        }
    }

    /// Stable key used in `settings.json`; never change one of these.
    pub fn id(self) -> &'static str {
        match self {
            Action::Forward => "forward",
            Action::Back => "back",
            Action::Left => "left",
            Action::Right => "right",
            Action::Jump => "jump",
            Action::Sneak => "sneak",
            Action::Sprint => "sprint",
            Action::ToggleSprint => "toggle_sprint",
            Action::ToggleSneak => "toggle_sneak",
            Action::Attack => "attack",
            Action::UseItem => "use_item",
            Action::PickBlock => "pick_block",
            Action::DropItem => "drop_item",
            Action::Hotbar(1) => "hotbar1",
            Action::Hotbar(2) => "hotbar2",
            Action::Hotbar(3) => "hotbar3",
            Action::Hotbar(4) => "hotbar4",
            Action::Hotbar(5) => "hotbar5",
            Action::Hotbar(6) => "hotbar6",
            Action::Hotbar(7) => "hotbar7",
            Action::Hotbar(8) => "hotbar8",
            Action::Hotbar(9) => "hotbar9",
            Action::Hotbar(_) => "hotbar0",
            Action::Inventory => "inventory",
            Action::Chat => "chat",
            Action::Command => "command",
            Action::Fullscreen => "fullscreen",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|action| action.id() == id)
    }

    /// Vanilla 1.8's defaults, which is also what the client hard-coded before
    /// bindings existed.
    pub const fn default_bind(self) -> Bind {
        match self {
            Action::Forward => Bind::Key(KeyCode::KeyW),
            Action::Back => Bind::Key(KeyCode::KeyS),
            Action::Left => Bind::Key(KeyCode::KeyA),
            Action::Right => Bind::Key(KeyCode::KeyD),
            Action::Jump => Bind::Key(KeyCode::Space),
            Action::Sneak => Bind::Key(KeyCode::ShiftLeft),
            Action::Sprint => Bind::Key(KeyCode::ControlLeft),
            // Not vanilla bindings: off until the player assigns a key.
            Action::ToggleSprint | Action::ToggleSneak => Bind::Unbound,
            Action::Attack => Bind::Mouse(MouseButton::Left),
            Action::UseItem => Bind::Mouse(MouseButton::Right),
            Action::PickBlock => Bind::Mouse(MouseButton::Middle),
            Action::DropItem => Bind::Key(KeyCode::KeyQ),
            Action::Hotbar(1) => Bind::Key(KeyCode::Digit1),
            Action::Hotbar(2) => Bind::Key(KeyCode::Digit2),
            Action::Hotbar(3) => Bind::Key(KeyCode::Digit3),
            Action::Hotbar(4) => Bind::Key(KeyCode::Digit4),
            Action::Hotbar(5) => Bind::Key(KeyCode::Digit5),
            Action::Hotbar(6) => Bind::Key(KeyCode::Digit6),
            Action::Hotbar(7) => Bind::Key(KeyCode::Digit7),
            Action::Hotbar(8) => Bind::Key(KeyCode::Digit8),
            Action::Hotbar(9) => Bind::Key(KeyCode::Digit9),
            Action::Hotbar(_) => Bind::Unbound,
            Action::Inventory => Bind::Key(KeyCode::KeyE),
            Action::Chat => Bind::Key(KeyCode::KeyT),
            Action::Command => Bind::Key(KeyCode::Slash),
            Action::Fullscreen => Bind::Key(KeyCode::F11),
        }
    }
}

/// The whole binding table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keybinds {
    binds: Vec<(Action, Bind)>,
}

impl Default for Keybinds {
    fn default() -> Self {
        Self {
            binds: Action::ALL
                .into_iter()
                .map(|action| (action, action.default_bind()))
                .collect(),
        }
    }
}

impl Keybinds {
    pub fn get(&self, action: Action) -> Bind {
        self.binds
            .iter()
            .find(|(known, _)| *known == action)
            .map(|(_, bind)| *bind)
            .unwrap_or(Bind::Unbound)
    }

    pub fn set(&mut self, action: Action, bind: Bind) {
        if let Some(entry) = self.binds.iter_mut().find(|(known, _)| *known == action) {
            entry.1 = bind;
        }
    }

    /// Bind `action` to `bind`. If another action already holds that binding it
    /// is unbound — vanilla lets the collision stand, but a silently duplicated
    /// key is a bug report waiting to happen — and the displaced action is
    /// returned so the screen can say which one lost its key.
    pub fn assign(&mut self, action: Action, bind: Bind) -> Option<Action> {
        let mut displaced = None;
        if bind != Bind::Unbound {
            for (other, current) in self.binds.iter_mut() {
                if *other != action && *current == bind {
                    *current = Bind::Unbound;
                    displaced = Some(*other);
                }
            }
        }
        self.set(action, bind);
        displaced
    }

    pub fn reset(&mut self, action: Action) {
        self.set(action, action.default_bind());
    }

    pub fn reset_all(&mut self) {
        *self = Self::default();
    }

    /// The action a key or button triggers, if any.
    pub fn action_for(&self, bind: Bind) -> Option<Action> {
        if bind == Bind::Unbound {
            return None;
        }
        self.binds
            .iter()
            .find(|(_, current)| *current == bind)
            .map(|(action, _)| *action)
    }

    /// Actions grouped by category, in display order.
    pub fn by_category(&self) -> Vec<(Category, Vec<Action>)> {
        Category::ALL
            .into_iter()
            .map(|category| {
                let actions = Action::ALL
                    .into_iter()
                    .filter(|action| action.category() == category)
                    .collect();
                (category, actions)
            })
            .collect()
    }
}

/// Display name for a key, vanilla-flavoured (`LSHIFT`, `SPACE`, `F3`).
pub fn key_name(code: KeyCode) -> &'static str {
    KEY_NAMES
        .iter()
        .find(|(known, _)| *known == code)
        .map(|(_, name)| *name)
        .unwrap_or("NONE")
}

/// Which key codes may be bound, and what they are called on screen.
///
/// `Escape` is deliberately absent: it is the "cancel rebinding"/"open menu"
/// key and cannot be stolen from that job.
pub const KEY_NAMES: &[(KeyCode, &str)] = &[
    (KeyCode::KeyA, "A"),
    (KeyCode::KeyB, "B"),
    (KeyCode::KeyC, "C"),
    (KeyCode::KeyD, "D"),
    (KeyCode::KeyE, "E"),
    (KeyCode::KeyF, "F"),
    (KeyCode::KeyG, "G"),
    (KeyCode::KeyH, "H"),
    (KeyCode::KeyI, "I"),
    (KeyCode::KeyJ, "J"),
    (KeyCode::KeyK, "K"),
    (KeyCode::KeyL, "L"),
    (KeyCode::KeyM, "M"),
    (KeyCode::KeyN, "N"),
    (KeyCode::KeyO, "O"),
    (KeyCode::KeyP, "P"),
    (KeyCode::KeyQ, "Q"),
    (KeyCode::KeyR, "R"),
    (KeyCode::KeyS, "S"),
    (KeyCode::KeyT, "T"),
    (KeyCode::KeyU, "U"),
    (KeyCode::KeyV, "V"),
    (KeyCode::KeyW, "W"),
    (KeyCode::KeyX, "X"),
    (KeyCode::KeyY, "Y"),
    (KeyCode::KeyZ, "Z"),
    (KeyCode::Digit0, "0"),
    (KeyCode::Digit1, "1"),
    (KeyCode::Digit2, "2"),
    (KeyCode::Digit3, "3"),
    (KeyCode::Digit4, "4"),
    (KeyCode::Digit5, "5"),
    (KeyCode::Digit6, "6"),
    (KeyCode::Digit7, "7"),
    (KeyCode::Digit8, "8"),
    (KeyCode::Digit9, "9"),
    (KeyCode::F1, "F1"),
    (KeyCode::F2, "F2"),
    (KeyCode::F3, "F3"),
    (KeyCode::F4, "F4"),
    (KeyCode::F5, "F5"),
    (KeyCode::F6, "F6"),
    (KeyCode::F7, "F7"),
    (KeyCode::F8, "F8"),
    (KeyCode::F9, "F9"),
    (KeyCode::F10, "F10"),
    (KeyCode::F11, "F11"),
    (KeyCode::F12, "F12"),
    (KeyCode::Space, "SPACE"),
    (KeyCode::Tab, "TAB"),
    (KeyCode::Enter, "ENTER"),
    (KeyCode::Backspace, "BACK"),
    (KeyCode::Delete, "DELETE"),
    (KeyCode::Insert, "INSERT"),
    (KeyCode::Home, "HOME"),
    (KeyCode::End, "END"),
    (KeyCode::PageUp, "PAGE UP"),
    (KeyCode::PageDown, "PAGE DOWN"),
    (KeyCode::ArrowUp, "UP"),
    (KeyCode::ArrowDown, "DOWN"),
    (KeyCode::ArrowLeft, "LEFT"),
    (KeyCode::ArrowRight, "RIGHT"),
    (KeyCode::ShiftLeft, "LSHIFT"),
    (KeyCode::ShiftRight, "RSHIFT"),
    (KeyCode::ControlLeft, "LCTRL"),
    (KeyCode::ControlRight, "RCTRL"),
    (KeyCode::AltLeft, "LALT"),
    (KeyCode::AltRight, "RALT"),
    (KeyCode::SuperLeft, "LWIN"),
    (KeyCode::SuperRight, "RWIN"),
    (KeyCode::CapsLock, "CAPS"),
    (KeyCode::Minus, "MINUS"),
    (KeyCode::Equal, "EQUALS"),
    (KeyCode::BracketLeft, "LBRACKET"),
    (KeyCode::BracketRight, "RBRACKET"),
    (KeyCode::Backslash, "BACKSLASH"),
    (KeyCode::Semicolon, "SEMICOLON"),
    (KeyCode::Quote, "APOSTROPHE"),
    (KeyCode::Backquote, "GRAVE"),
    (KeyCode::Comma, "COMMA"),
    (KeyCode::Period, "PERIOD"),
    (KeyCode::Slash, "SLASH"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_what_the_client_did_before_bindings() {
        let binds = Keybinds::default();
        assert_eq!(binds.get(Action::Forward), Bind::Key(KeyCode::KeyW));
        assert_eq!(binds.get(Action::Sneak), Bind::Key(KeyCode::ShiftLeft));
        assert_eq!(binds.get(Action::Sprint), Bind::Key(KeyCode::ControlLeft));
        assert_eq!(binds.get(Action::Attack), Bind::Mouse(MouseButton::Left));
        assert_eq!(binds.get(Action::UseItem), Bind::Mouse(MouseButton::Right));
        assert_eq!(binds.get(Action::Inventory), Bind::Key(KeyCode::KeyE));
        assert_eq!(binds.get(Action::Hotbar(4)), Bind::Key(KeyCode::Digit4));
        // The latched toggles are opt-in, so a fresh install has no key for them.
        assert_eq!(binds.get(Action::ToggleSprint), Bind::Unbound);
        assert_eq!(binds.get(Action::ToggleSneak), Bind::Unbound);
        assert_eq!(Action::ToggleSprint.category(), Category::Movement);
    }

    #[test]
    fn every_action_has_a_unique_id_and_a_label() {
        let mut ids: Vec<&str> = Action::ALL.iter().map(|action| action.id()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), Action::ALL.len());
        for action in Action::ALL {
            assert_eq!(Action::from_id(action.id()), Some(action));
            assert!(!action.label().is_empty());
        }
    }

    #[test]
    fn binding_a_taken_key_displaces_the_old_owner() {
        let mut binds = Keybinds::default();
        // Q is Drop Item; give it to Attack and expect Drop to lose it.
        let displaced = binds.assign(Action::Attack, Bind::Key(KeyCode::KeyQ));
        assert_eq!(displaced, Some(Action::DropItem));
        assert_eq!(binds.get(Action::Attack), Bind::Key(KeyCode::KeyQ));
        assert_eq!(binds.get(Action::DropItem), Bind::Unbound);
        assert_eq!(
            binds.action_for(Bind::Key(KeyCode::KeyQ)),
            Some(Action::Attack)
        );
    }

    #[test]
    fn rebinding_attack_away_from_the_mouse_clears_the_mouse_binding() {
        let mut binds = Keybinds::default();
        binds.assign(Action::Attack, Bind::Key(KeyCode::KeyR));
        assert_eq!(binds.action_for(Bind::Mouse(MouseButton::Left)), None);
        assert_eq!(binds.get(Action::Attack), Bind::Key(KeyCode::KeyR));
    }

    #[test]
    fn unbound_actions_never_match() {
        let mut binds = Keybinds::default();
        binds.assign(Action::DropItem, Bind::Unbound);
        assert_eq!(binds.action_for(Bind::Unbound), None);
        assert_eq!(binds.get(Action::DropItem), Bind::Unbound);
    }

    #[test]
    fn reset_restores_one_row_then_all_of_them() {
        let mut binds = Keybinds::default();
        binds.assign(Action::Forward, Bind::Key(KeyCode::ArrowUp));
        binds.reset(Action::Forward);
        assert_eq!(binds.get(Action::Forward), Bind::Key(KeyCode::KeyW));
        binds.assign(Action::Forward, Bind::Key(KeyCode::ArrowUp));
        binds.reset_all();
        assert_eq!(binds, Keybinds::default());
    }

    #[test]
    fn bindings_encode_and_decode() {
        for bind in [
            Bind::Key(KeyCode::KeyW),
            Bind::Key(KeyCode::F11),
            Bind::Mouse(MouseButton::Left),
            Bind::Mouse(MouseButton::Middle),
            Bind::Unbound,
        ] {
            assert_eq!(Bind::decode(&bind.encode()), bind, "{}", bind.encode());
        }
        // Unknown text degrades to unbound rather than to a wrong key.
        assert_eq!(Bind::decode("key:NotAKey"), Bind::Unbound);
        assert_eq!(Bind::decode("mouse:Sixteen"), Bind::Unbound);
        assert_eq!(Bind::decode("garbage"), Bind::Unbound);
    }

    #[test]
    fn escape_is_not_bindable() {
        assert_eq!(key_name(KeyCode::Escape), "NONE");
        assert!(!KEY_NAMES.iter().any(|(code, _)| *code == KeyCode::Escape));
    }

    #[test]
    fn every_category_has_at_least_one_row() {
        let binds = Keybinds::default();
        for (category, actions) in binds.by_category() {
            assert!(!actions.is_empty(), "{category:?} is empty");
        }
        let total: usize = binds
            .by_category()
            .iter()
            .map(|(_, actions)| actions.len())
            .sum();
        assert_eq!(total, Action::ALL.len());
    }

    #[test]
    fn display_names_read_like_a_keyboard() {
        assert_eq!(Bind::Mouse(MouseButton::Left).name(), "Button 1");
        assert_eq!(Bind::Key(KeyCode::Space).name(), "SPACE");
        assert_eq!(Bind::Key(KeyCode::ShiftLeft).name(), "LSHIFT");
        assert_eq!(Bind::Key(KeyCode::F3).name(), "F3");
        assert_eq!(Bind::Unbound.name(), "NONE");
    }
}
