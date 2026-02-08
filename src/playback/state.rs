/// Playback state for the time-shift engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PlaybackState {
    /// Read follows write at base_delay offset. Both advance in lockstep.
    Live = 0,
    /// Write continues advancing. Read is frozen. Buffer fills up.
    Paused = 1,
    /// Both advance, but read is behind write by a variable amount.
    TimeShifted = 2,
}

impl PlaybackState {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Live,
            1 => Self::Paused,
            2 => Self::TimeShifted,
            _ => Self::Live,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Live => "LIVE",
            Self::Paused => "PAUSED",
            Self::TimeShifted => "TIME-SHIFTED",
        }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            Self::Live => ">>",
            Self::Paused => "||",
            Self::TimeShifted => "> ",
        }
    }
}
