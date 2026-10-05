//! Synthol patch: runtime-switchable keyboard capture.
//!
//! By default the editor forwards no keys to Slint so the host keeps
//! receiving them (e.g. a DAW's computer-MIDI keyboard). Plugins that
//! occasionally need keys - a text field in a modal dialog - share a
//! [`KeyboardCapture`] handle with the editor and flip its mode while the
//! dialog is open.
//!
//! Any mode other than [`KeyboardCaptureMode::None`] also tells the editor
//! that a modal overlay is up, so it repaints whole frames meanwhile (see
//! `platform::render_to_rgba`).

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

/// Which keyboard events the editor forwards to Slint (and consumes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyboardCaptureMode {
    /// Every key goes back to the host.
    #[default]
    None,
    /// Only Escape is forwarded to Slint; every other key goes to the host.
    Escape,
    /// Every key is forwarded to Slint.
    All,
}

impl KeyboardCaptureMode {
    fn to_u8(self) -> u8 {
        match self {
            Self::None => 0,
            Self::Escape => 1,
            Self::All => 2,
        }
    }

    fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Escape,
            2 => Self::All,
            _ => Self::None,
        }
    }
}

/// Shared, thread-safe handle to the editor's current [`KeyboardCaptureMode`].
#[derive(Clone, Debug, Default)]
pub struct KeyboardCapture(Arc<AtomicU8>);

impl KeyboardCapture {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self, mode: KeyboardCaptureMode) {
        self.0.store(mode.to_u8(), Ordering::Relaxed);
    }

    #[must_use]
    pub fn get(&self) -> KeyboardCaptureMode {
        KeyboardCaptureMode::from_u8(self.0.load(Ordering::Relaxed))
    }
}
