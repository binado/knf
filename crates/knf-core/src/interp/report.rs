//! Recording problems, and the proof that one was recorded.

use super::{Problem, Problems};

/// Proof that a problem is in the [`Sink`]. Only [`Sink::report`] makes one,
/// so a failed result can be cached and passed on without reporting it again.
#[derive(Debug, Clone, Copy)]
pub(super) struct Reported(());

/// A resolution result: the value, or a failure already in the [`Sink`].
pub(super) type Outcome<T> = Result<T, Reported>;

#[derive(Default)]
pub(super) struct Sink(Vec<Problem>);

impl Sink {
    pub(super) fn report(&mut self, problem: Problem) -> Reported {
        self.0.push(problem);
        Reported(())
    }

    /// Every problem reported, or the resolved value if there were none.
    ///
    /// A value can resolve while problems exist: an object merged over a
    /// broken reference does not take its failure.
    pub(super) fn finish<T>(self, outcome: Outcome<T>) -> Result<T, Problems> {
        match Problems::new(self.0) {
            Some(problems) => Err(problems),
            None => Ok(outcome.expect("a failed outcome has a reported problem")),
        }
    }
}
