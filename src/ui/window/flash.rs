//! Brief header-button feedback flashes, as plain state so their generation
//! logic is unit-tested without a display: the Diff Mode button's "No Changes"
//! label and the copy button's checkmark.
//!
//! Each flash gets a generation number. Only the timer of the newest flash may
//! end it, so an older timer never cuts a newer flash short.

use std::cell::Cell;

#[derive(Default)]
pub(super) struct FlashState {
    generation: Cell<u32>,
    active: Cell<bool>,
}

impl FlashState {
    /// Start a flash and return its generation.
    pub(super) fn begin(&self) -> u32 {
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        self.active.set(true);
        generation
    }

    /// End the flash `generation` started. Returns false (changing nothing)
    /// when a newer flash has replaced it.
    pub(super) fn end(&self, generation: u32) -> bool {
        if self.generation.get() == generation {
            self.active.set(false);
            true
        } else {
            false
        }
    }

    /// True while a flash owns the button.
    pub(super) fn is_active(&self) -> bool {
        self.active.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flash_ends_with_its_own_generation() {
        let flash = FlashState::default();
        assert!(!flash.is_active());
        let first = flash.begin();
        assert!(flash.is_active());
        assert!(flash.end(first));
        assert!(!flash.is_active());
    }

    #[test]
    fn an_older_timer_cannot_end_a_newer_flash() {
        let flash = FlashState::default();
        let first = flash.begin();
        let second = flash.begin();
        assert!(!flash.end(first));
        assert!(flash.is_active());
        assert!(flash.end(second));
        assert!(!flash.is_active());
    }

    #[test]
    fn generations_wrap() {
        let flash = FlashState::default();
        flash.generation.set(u32::MAX);
        let generation = flash.begin();
        assert_eq!(generation, 0);
        assert!(flash.end(generation));
    }
}
