//! Spawning hyper's tasks on geario.

use std::future::Future;

/// Runs hyper's connection tasks on the current worker.
///
/// geario is thread-per-core and its handles are `Rc`-based, so tasks stay on
/// the worker that accepted the connection. That is the property being kept;
/// an executor that moved work between threads would give it away.
#[derive(Clone, Copy, Debug, Default)]
pub struct GearioExecutor;

impl<F> hyper::rt::Executor<F> for GearioExecutor
where
    F: Future + 'static,
    F::Output: 'static,
{
    fn execute(&self, fut: F) {
        geario::rt::spawn(async move {
            fut.await;
        });
    }
}
