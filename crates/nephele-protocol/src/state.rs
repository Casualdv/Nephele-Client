use std::fmt;

/// The four connection states of the Minecraft Java protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum State {
    Handshake,
    Status,
    Login,
    Play,
}

impl State {
    pub fn name(&self) -> &'static str {
        match self {
            State::Handshake => "handshake",
            State::Status => "status",
            State::Login => "login",
            State::Play => "play",
        }
    }
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Value carried by the handshake packet's `next state` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum NextState {
    Status = 1,
    Login = 2,
}

impl NextState {
    pub fn to_state(self) -> State {
        match self {
            NextState::Status => State::Status,
            NextState::Login => State::Login,
        }
    }
}
