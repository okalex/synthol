use std::sync::Arc;

use truce::prelude::*;

mod editor;
pub mod engine;
pub mod plugin;

pub use plugin::{Synth, SynthParams};

truce::plugin! {
    logic: Synth,
    params: SynthParams,
}

#[cfg(test)]
mod tests;
