//! The async bridge for one-shot blocking capture workers.
//!
//! Every operation of this backend runs its blocking `x11rb` chain on a
//! short-lived worker thread holding a one-shot connection, bridged into a
//! runtime-agnostic future - the `flowshot-capture-wayland` worker pattern,
//! pinned to the single [`X11Error`] family (no generic backend-error
//! parameter: X11 has exactly one error type, tagged
//! [`BackendKind::X11`]).

use std::future::Future;
use std::thread::Builder;
use std::time::Duration;

use flowshot_capture::{BackendKind, CaptureError};
use futures::future::{Either, ready};
use futures::{FutureExt, StreamExt};

use crate::error::X11Error;

/// Deadline for one bridged worker operation: a hung X server surfaces
/// [`CaptureError::Timeout`] instead of parking the caller forever - the
/// same 10 s bound the Wayland sibling applies to its capture thread
/// (`thread.rs` `REPLY_TIMEOUT`).
const WORKER_DEADLINE: Duration = Duration::from_secs(10);

/// Runs one blocking X11 operation on a dedicated worker thread and bridges
/// the result into a non-blocking future bounded by [`WORKER_DEADLINE`].
///
/// The worker owns the one-shot connection; if the returned future is dropped
/// (caller cancelled), the worker still runs to completion and the connection
/// teardown releases every server-side resource - the result is simply
/// discarded. A watchdog thread races the worker over an mpsc channel: the
/// first event wins, so a worker that hangs (an X server that accepts then
/// stops answering) or dies without sending (a panic - its sender drops, but
/// the watchdog's keeps the stream open) surfaces as a typed
/// [`CaptureError::Timeout`] at the deadline rather than a hang. The watchdog
/// is detached and lives at most one deadline of sleep; a failed watchdog
/// spawn degrades to the unbounded bridge with a debug log. A timing test is
/// deliberately absent: it would need a real 10 s hang (the repo bans
/// fixed-sleep tests).
pub(crate) fn spawn_worker<T, F>(
    name: &str,
    work: F,
) -> impl Future<Output = Result<T, CaptureError>> + Send
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, X11Error> + Send + 'static,
{
    let (events, winner) = futures::channel::mpsc::unbounded();
    let watchdog_events = events.clone();
    let spawned = Builder::new().name(name.to_owned()).spawn(move || {
        let result = work().map_err(CaptureError::from);
        if events.unbounded_send(result).is_err() {
            tracing::debug!("capture result discarded: the requesting future was cancelled");
        }
    });
    match spawned {
        Err(error) => Either::Right(ready(Err(CaptureError::Backend {
            backend: BackendKind::X11,
            source: X11Error::from(error).into(),
        }))),
        Ok(_worker) => {
            let watchdog = Builder::new()
                .name("flowshot-x11-watchdog".to_owned())
                .spawn(move || {
                    std::thread::sleep(WORKER_DEADLINE);
                    let timeout = CaptureError::from(X11Error::Timeout {
                        timeout: WORKER_DEADLINE,
                    });
                    let _lost_race = watchdog_events.unbounded_send(Err(timeout));
                });
            if let Err(error) = watchdog {
                tracing::debug!(
                    "watchdog thread unavailable; the worker bridge runs without a deadline: {error}"
                );
            }
            Either::Left(winner.into_future().map(|(received, _rest)| {
                match received {
                    Some(result) => result,
                    None => Err(CaptureError::Backend {
                        backend: BackendKind::X11,
                        source: X11Error::Internal(
                            "the capture worker ended without a result (panic?)",
                        )
                        .into(),
                    }),
                }
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use futures::executor::block_on;

    use super::*;

    #[test]
    fn worker_bridges_the_success_value() {
        let result = block_on(spawn_worker("test-ok", || Ok(7u32)));
        assert_eq!(result.unwrap(), 7);
    }

    #[test]
    fn worker_bridges_typed_errors_tagged_x11() {
        let result = block_on(spawn_worker("test-err", || -> Result<(), X11Error> {
            Err(X11Error::MissingExtension { name: "RANDR" })
        }));
        match result.unwrap_err() {
            CaptureError::Backend { backend, source } => {
                assert_eq!(backend, BackendKind::X11);
                assert!(source.to_string().contains("RANDR"), "{source}");
            }
            other => panic!("expected CaptureError::Backend, got {other:?}"),
        }
    }

    #[test]
    fn timeout_errors_keep_their_mapping() {
        let result = block_on(spawn_worker("test-timeout", || -> Result<(), X11Error> {
            Err(X11Error::Timeout {
                timeout: std::time::Duration::from_secs(1),
            })
        }));
        assert!(matches!(
            result.unwrap_err(),
            CaptureError::Timeout {
                backend: BackendKind::X11
            }
        ));
    }
}
