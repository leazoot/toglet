//! How far a switch has actually got.
//!
//! The panel shows four steps: Check → Switch → Verify → Ready. Nothing in this file reads a
//! clock, and a step can only be recorded once the one before it is done, so there is no way to
//! express the progress bar that walks forward on a timer while the work is stuck - which is
//! the specific lie this project refuses to tell.

use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};

/// The four steps, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SwitchStep {
    /// Pre-checks.
    Check,
    /// Backup and atomic replacement.
    Switch,
    /// The replaced authentication is read back and the identity compared.
    Verify,
    /// Verified, and the active account recorded.
    Ready,
}

impl SwitchStep {
    /// The step's position, 1 to 4.
    pub fn number(self) -> u8 {
        match self {
            Self::Check => 1,
            Self::Switch => 2,
            Self::Verify => 3,
            Self::Ready => 4,
        }
    }

    fn next(self) -> Option<Self> {
        match self {
            Self::Check => Some(Self::Switch),
            Self::Switch => Some(Self::Verify),
            Self::Verify => Some(Self::Ready),
            Self::Ready => None,
        }
    }
}

/// Told when a step really finished.
///
/// The point of the seam is what it *cannot* do: it is only ever called from the same place that
/// records the step in [`SwitchProgress`], and that refuses anything but the next step. So an
/// observer cannot be driven forward by a timer while the work is stuck. Nothing is passed
/// back, so an observer cannot influence the switch either.
pub trait StepObserver {
    fn completed(&self, step: SwitchStep);
}

/// The observer used when nobody is watching.
pub struct NoObserver;

impl StepObserver for NoObserver {
    fn completed(&self, _: SwitchStep) {}
}

/// What a switch has finished so far.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SwitchProgress {
    completed: Option<SwitchStep>,
}

impl SwitchProgress {
    pub fn new() -> Self {
        Self::default()
    }

    /// The last step that actually finished.
    pub fn completed(&self) -> Option<SwitchStep> {
        self.completed
    }

    /// How many steps are done, 0 to 4 - the number the panel renders.
    pub fn number(&self) -> u8 {
        self.completed.map_or(0, SwitchStep::number)
    }

    /// Records that `step` finished.
    ///
    /// Refuses anything but the next step. Skipping one would mean reporting work as done that
    /// nothing performed, and repeating one would mean a step ran twice; both are bugs in the
    /// caller rather than conditions a user can cause, which is why this is `Internal` and not
    /// retryable.
    pub fn complete(&mut self, step: SwitchStep, phase: Phase) -> Result<()> {
        let expected = match self.completed {
            None => Some(SwitchStep::Check),
            Some(done) => done.next(),
        };

        if expected != Some(step) {
            return Err(
                TogletError::new(ErrorCode::Internal, phase, false, UserAction::None)
                    .with_detail("a switch step was recorded out of order"),
            );
        }

        self.completed = Some(step);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PHASE: Phase = Phase::Precheck;

    #[test]
    fn a_switch_that_has_not_started_shows_no_progress() {
        assert_eq!(SwitchProgress::new().number(), 0);
        assert_eq!(SwitchProgress::new().completed(), None);
    }

    #[test]
    fn the_four_steps_are_recorded_in_order() {
        let mut progress = SwitchProgress::new();

        for (step, number) in [
            (SwitchStep::Check, 1),
            (SwitchStep::Switch, 2),
            (SwitchStep::Verify, 3),
            (SwitchStep::Ready, 4),
        ] {
            progress.complete(step, PHASE).expect("the step is next");
            assert_eq!(progress.number(), number);
        }
    }

    #[test]
    fn a_step_cannot_be_skipped() {
        let mut progress = SwitchProgress::new();

        let error = progress
            .complete(SwitchStep::Ready, PHASE)
            .expect_err("reporting the last step first must be refused");

        assert_eq!(error.code(), ErrorCode::Internal);
        assert_eq!(
            progress.number(),
            0,
            "a refused step must not move the progress it refused to accept"
        );
    }

    #[test]
    fn verification_cannot_be_skipped_on_the_way_to_ready() {
        // The one that matters: "Ready" without "Verify" is a switch reported as successful
        // without anybody having checked it took effect.
        let mut progress = SwitchProgress::new();
        progress.complete(SwitchStep::Check, PHASE).expect("first");
        progress
            .complete(SwitchStep::Switch, PHASE)
            .expect("second");

        assert!(progress.complete(SwitchStep::Ready, PHASE).is_err());
        assert_eq!(progress.number(), 2);
    }

    #[test]
    fn a_step_cannot_be_recorded_twice() {
        let mut progress = SwitchProgress::new();
        progress.complete(SwitchStep::Check, PHASE).expect("first");

        assert!(progress.complete(SwitchStep::Check, PHASE).is_err());
    }

    #[test]
    fn nothing_here_advances_without_being_told_to() {
        // There is no clock input, so the only way progress moves is a completed step. Asserted
        // by construction rather than by waiting.
        let progress = SwitchProgress::new();

        assert_eq!(progress.number(), 0);
        assert_eq!(progress.number(), 0);
    }
}
