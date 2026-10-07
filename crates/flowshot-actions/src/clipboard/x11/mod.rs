//! X11 clipboard backend: ICCCM `CLIPBOARD` selection owner.
//!
//! [`X11Clipboard`] mirrors the daemon-ownership model of
//! [`super::WaylandClipboard`]: [`ClipboardBackend::serve`] establishes
//! selection ownership and then serves the offer from a dedicated thread
//! inside the CALLING process, so an offer served by the daemon outlives
//! the capturing UI process and dies with the daemon (documented
//! behavior).
//!
//! # Protocol notes
//!
//! - Ownership: a tiny `InputOnly` window acts as the selection owner.
//!   A daemon context has no triggering user event, so the ICCCM
//!   timestamp for `SetSelectionOwner` comes from the zero-length
//!   property-append trick: appending zero bytes to a private property
//!   makes the server emit a `PropertyNotify` carrying a valid server
//!   timestamp (`CurrentTime` is not acceptable for selection
//!   ownership).
//! - Serving: `SelectionRequest` events are answered with `TARGETS`
//!   (protocol targets, every offered MIME, and the automatic text
//!   variants for `text/plain` — parity with the Wayland backend),
//!   `TIMESTAMP`, `MULTIPLE` (ICCCM-required), and the offer's data
//!   targets. All MIME entries of one offer stay servable
//!   simultaneously.
//! - INCR: payloads above the chunk budget (server maximum request size
//!   minus overhead, capped at 256 KiB) are transferred incrementally:
//!   the property is first set to type `INCR` carrying the total size,
//!   then one chunk is written per `PropertyNotify(Delete)` from the
//!   requestor, and a zero-length write terminates the transfer.
//! - Supersede: a second `serve()` spawns a new owner thread whose
//!   `SetSelectionOwner` makes the server send `SelectionClear` to the
//!   previous one, which exits quietly. Losing ownership to ANY other
//!   client is likewise normal operation, not an error.
//! - Loss reporting: an exiting serving thread fires the optional
//!   [`OfferLossHook`](super::OfferLossHook) iff its ownership claim was
//!   still the process's most recent one — a process-wide epoch counter,
//!   claimed by every `serve()` BEFORE the new owner takes the selection,
//!   keeps self-supersede silent. The hook therefore means "the offer was
//!   lost to another client (or the connection died)", never "we replaced
//!   it ourselves" — the distinction the daemon's hold-release wiring
//!   depends on.
//! - Connection: the serving thread owns its own [`RustConnection`];
//!   connections are not shared across threads in this pattern.

// allow: SIZE_OK — what remains after the `pure` (headless-testable
// composition, INCR math, atom-pair decode) and `wire` (atom interning,
// offer construction, property writes) extractions is the one indivisible
// ICCCM protocol state machine: selection ownership, timestamp
// acquisition, the epoch / loss-report machinery, SelectionRequest
// dispatch, sender-side INCR and MULTIPLE, all sharing one OwnerState.
// Further splitting would scatter a single protocol flow without
// behavioral gain. Reassess if this file grows again.

mod pure;
mod wire;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use x11rb::connection::{Connection, RequestConnection as _};
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    Atom, ChangeWindowAttributesAux, ConnectionExt, CreateWindowAux, EventMask, PropMode, Property,
    SelectionNotifyEvent, SelectionRequestEvent, Timestamp, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;

use super::offer::ClipboardOffer;
use super::{ClipboardBackend, OfferLossHook};
use crate::error::ClipboardError;
use pure::{PROTOCOL_TARGETS, atom_pairs, incr_chunk_size, next_incr_slice};
use wire::{build_offer, change_property8, change_property32, intern};

/// Deadline for the timestamp `PropertyNotify` to arrive.
const TIMESTAMP_TIMEOUT: Duration = Duration::from_secs(5);
/// Poll interval while waiting for the timestamp `PropertyNotify`.
const POLL_INTERVAL: Duration = Duration::from_millis(1);
/// `GetProperty` length cap (in 4-byte units) for a `MULTIPLE` pair list.
const MULTIPLE_LIST_LIMIT: u32 = 4096;
/// Name of the dedicated serving thread.
const THREAD_NAME: &str = "flowshot-x11-clipboard";

/// Process-wide ownership epoch: every `serve()` claim (across ALL
/// backend instances — the daemon builds a fresh one per capture) bumps
/// this before its owner takes the selection, and an exiting serving
/// thread reports loss through its hook only while its claim is still the
/// latest. A superseding `serve()` therefore silences the previous
/// thread's exit (the server's `SelectionClear` reaches it only AFTER the
/// bump), and the hook fires exclusively for a genuine external takeover
/// or connection death.
static OWNER_EPOCH: AtomicU64 = AtomicU64::new(0);

/// Claims the next ownership epoch (one `serve()` = one claim).
fn next_owner_epoch() -> u64 {
    OWNER_EPOCH.fetch_add(1, Ordering::AcqRel) + 1
}

/// Rewinds a claimed-but-never-established epoch (a `serve()` whose setup
/// failed): without the rewind, the dead claim would silence the
/// STILL-LIVE previous owner's genuine later loss (its
/// [`report_loss_if_latest`] check would see the newer epoch), pinning a
/// daemon's clipboard-offer hold until killed. The compare-exchange
/// rewinds only when no newer claim happened in between; a newer claim
/// belongs to a `serve()` that either succeeds (genuinely superseding the
/// previous owner, whose silence is then correct) or fails and rewinds
/// itself.
fn unclaim_owner_epoch(epoch: u64) {
    let _rewound =
        OWNER_EPOCH.compare_exchange(epoch, epoch - 1, Ordering::AcqRel, Ordering::Acquire);
}

/// Serving-thread exit report: fires `hook` iff `epoch` is still the
/// latest claim (self-supersede exits silently — see [`OWNER_EPOCH`]).
fn report_loss_if_latest(hook: Option<&OfferLossHook>, epoch: u64) {
    let Some(hook) = hook else {
        return;
    };
    if OWNER_EPOCH.load(Ordering::Acquire) == epoch {
        hook();
    }
}

/// Production X11 backend: daemon-owned `CLIPBOARD` selection offers.
///
/// Each [`ClipboardBackend::serve`] call supersedes the previous offer:
/// the new owner thread takes the selection, the server sends
/// `SelectionClear` to the previous thread, and that thread exits (its
/// record here is replaced and the handle detached). The offer lives as
/// long as this process holds the selection; process exit destroys it
/// (same lifecycle as [`super::WaylandClipboard`]).
pub struct X11Clipboard {
    current: Mutex<Option<Serving>>,
    loss_hook: Option<OfferLossHook>,
}

impl std::fmt::Debug for X11Clipboard {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("X11Clipboard")
            .field("current", &self.current)
            .field("loss_hook", &self.loss_hook.as_ref().map(|_| "set"))
            .finish()
    }
}

/// Record of the live serving thread (replaced on each `serve()`).
#[derive(Debug)]
struct Serving {
    window: Window,
    thread: JoinHandle<()>,
}

impl X11Clipboard {
    /// Create the X11 backend (no connection until [`serve`](ClipboardBackend::serve)).
    #[must_use]
    pub fn new() -> Self {
        Self {
            current: Mutex::new(None),
            loss_hook: None,
        }
    }

    /// Create the X11 backend with an offer-loss hook: fired from the
    /// serving thread when the most recently served offer loses
    /// `CLIPBOARD` ownership to another client (`SelectionClear`) or the
    /// serving connection dies — never for a self-supersede (see
    /// [`OfferLossHook`](super::OfferLossHook)).
    #[must_use]
    pub fn with_loss_hook(hook: OfferLossHook) -> Self {
        Self {
            current: Mutex::new(None),
            loss_hook: Some(hook),
        }
    }
}

impl Default for X11Clipboard {
    fn default() -> Self {
        Self::new()
    }
}

impl ClipboardBackend for X11Clipboard {
    /// Take `CLIPBOARD` ownership and serve `offer` from a dedicated
    /// thread in this process.
    ///
    /// Returns once ownership is established (or fails); serving then
    /// continues in the background until another client takes the
    /// selection. A previous offer from this backend is superseded.
    ///
    /// # Errors
    ///
    /// [`ClipboardError::Transport`] for connection/protocol failures,
    /// [`ClipboardError::X11`] when the ICCCM timestamp cannot be
    /// obtained or the server refuses ownership.
    fn serve(&self, offer: ClipboardOffer) -> Result<(), ClipboardError> {
        // Claim the epoch BEFORE the new owner takes the selection: the
        // server delivers SelectionClear to the previous thread only after
        // SetSelectionOwner below, so the previous thread's exit always
        // sees a newer claim and stays silent (no spurious loss report).
        let epoch = next_owner_epoch();
        let (ready_tx, ready_rx) = mpsc::channel();
        let hook = self.loss_hook.clone();
        let thread = match std::thread::Builder::new()
            .name(THREAD_NAME.to_owned())
            .spawn(move || run_owner(offer, ready_tx, hook, epoch))
        {
            Ok(thread) => thread,
            Err(error) => {
                unclaim_owner_epoch(epoch);
                return Err(ClipboardError::transport(error));
            }
        };
        let window = match ready_rx
            .recv()
            .map_err(|_: RecvError| {
                ClipboardError::X11(
                    "the serving thread exited before reporting ownership".to_owned(),
                )
            })
            .flatten()
        {
            Ok(window) => window,
            Err(error) => {
                // The claim never became a live ownership: rewind it so a
                // still-live previous owner's genuine later loss is not
                // silenced by the dead claim.
                unclaim_owner_epoch(epoch);
                drop(thread);
                return Err(error);
            }
        };
        let previous = self
            .current
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .replace(Serving { window, thread });
        if let Some(Serving { window, thread }) = previous {
            // Detach: the server already sent it SelectionClear when the
            // new owner above took the selection, so it exits by itself.
            tracing::debug!(window, "superseding previous x11 clipboard offer");
            drop(thread);
        }
        Ok(())
    }
}

/// Serving-thread body: establish ownership, report, then serve; on exit
/// (ownership lost or connection dead) report the loss through the hook
/// when this claim was still the latest.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the Sender must be owned by the thread: its drop on exit is what unblocks a waiting serve() with the thread-exited error"
)]
fn run_owner(
    offer: ClipboardOffer,
    ready: mpsc::Sender<Result<Window, ClipboardError>>,
    hook: Option<OfferLossHook>,
    epoch: u64,
) {
    let state = match setup_owner(offer) {
        Ok(state) => state,
        Err(err) => {
            // Ownership was never established: there is no offer to lose,
            // so the exit stays silent.
            if ready.send(Err(err)).is_err() {
                tracing::warn!(
                    "clipboard serve() caller gone before the ownership error could be reported"
                );
            }
            return;
        }
    };
    if ready.send(Ok(state.window)).is_err() {
        tracing::warn!("clipboard serve() caller gone; serving until ownership is lost anyway");
    }
    tracing::debug!(window = state.window, "x11 clipboard ownership acquired");
    state.event_loop();
    report_loss_if_latest(hook.as_ref(), epoch);
}

/// Interned atoms the serving loop needs.
struct Atoms {
    clipboard: Atom,
    targets: Atom,
    timestamp: Atom,
    multiple: Atom,
    atom_pair: Atom,
    incr: Atom,
    atom: Atom,
    integer: Atom,
    ts_probe: Atom,
}

impl Atoms {
    fn intern_all<C: ConnectionExt>(conn: &C) -> Result<Self, ClipboardError> {
        Ok(Self {
            clipboard: intern(conn, b"CLIPBOARD")?,
            targets: intern(conn, b"TARGETS")?,
            timestamp: intern(conn, b"TIMESTAMP")?,
            multiple: intern(conn, b"MULTIPLE")?,
            atom_pair: intern(conn, b"ATOM_PAIR")?,
            incr: intern(conn, b"INCR")?,
            atom: intern(conn, b"ATOM")?,
            integer: intern(conn, b"INTEGER")?,
            ts_probe: intern(conn, b"FLOWSHOT_CLIPBOARD_TS")?,
        })
    }
}

/// One servable target: the atom a requestor asks for, the property type
/// the answer is written under (convention: the target atom itself), and
/// the payload.
struct ServedTarget {
    target: Atom,
    prop_type: Atom,
    data: Arc<[u8]>,
}

/// The interned, alias-expanded form of a [`ClipboardOffer`].
struct ServedOffer {
    targets: Vec<ServedTarget>,
}

/// A running INCR transfer to one (requestor, property) pair.
#[derive(Clone)]
struct IncrTransfer {
    data: Arc<[u8]>,
    offset: usize,
    chunk: usize,
    prop_type: Atom,
}

/// Established selection owner: connection, atoms, offer, and the INCR
/// chunk budget. Lives entirely on the serving thread.
struct OwnerState {
    conn: RustConnection,
    atoms: Atoms,
    served: ServedOffer,
    window: Window,
    time: Timestamp,
    chunk: usize,
}

/// Connect, create the owner window, acquire a timestamp, and take the
/// selection. Every failure before ownership is a typed error.
fn setup_owner(offer: ClipboardOffer) -> Result<OwnerState, ClipboardError> {
    let (conn, screen) = x11rb::connect(None).map_err(ClipboardError::transport)?;
    let atoms = Atoms::intern_all(&conn)?;
    let served = build_offer(&conn, offer)?;
    let root = conn
        .setup()
        .roots
        .get(screen)
        .ok_or_else(|| ClipboardError::X11(format!("display has no screen {screen}")))?
        .root;
    let window = conn.generate_id().map_err(ClipboardError::transport)?;
    conn.create_window(
        0,
        window,
        root,
        0,
        0,
        1,
        1,
        0,
        WindowClass::INPUT_ONLY,
        x11rb::COPY_FROM_PARENT,
        &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
    )
    .map_err(ClipboardError::transport)?
    .check()
    .map_err(ClipboardError::transport)?;
    let time = acquire_timestamp(&conn, window, atoms.ts_probe)?;
    conn.set_selection_owner(window, atoms.clipboard, time)
        .map_err(ClipboardError::transport)?
        .check()
        .map_err(ClipboardError::transport)?;
    conn.flush().map_err(ClipboardError::transport)?;
    let owner = conn
        .get_selection_owner(atoms.clipboard)
        .map_err(ClipboardError::transport)?
        .reply()
        .map_err(ClipboardError::transport)?
        .owner;
    if owner != window {
        return Err(ClipboardError::X11(
            "server refused CLIPBOARD selection ownership".to_owned(),
        ));
    }
    let chunk = incr_chunk_size(conn.maximum_request_bytes());
    Ok(OwnerState {
        conn,
        atoms,
        served,
        window,
        time,
        chunk,
    })
}

/// Obtain a valid ICCCM timestamp without a triggering user event:
/// append zero bytes to a private property on `window` (which must
/// already select [`EventMask::PROPERTY_CHANGE`]) and take the server
/// timestamp from the resulting `PropertyNotify`.
fn acquire_timestamp<C: Connection>(
    conn: &C,
    window: Window,
    probe: Atom,
) -> Result<Timestamp, ClipboardError> {
    conn.change_property(PropMode::APPEND, window, probe, probe, 8, 0, &[])
        .map_err(ClipboardError::transport)?
        .check()
        .map_err(ClipboardError::transport)?;
    conn.flush().map_err(ClipboardError::transport)?;
    let deadline = Instant::now() + TIMESTAMP_TIMEOUT;
    loop {
        match conn.poll_for_event().map_err(ClipboardError::transport)? {
            Some(Event::PropertyNotify(notify))
                if notify.window == window && notify.state == Property::NEW_VALUE =>
            {
                return Ok(notify.time);
            }
            Some(_) => {}
            None => {
                if Instant::now() >= deadline {
                    return Err(ClipboardError::X11(
                        "timed out waiting for the timestamp PropertyNotify".to_owned(),
                    ));
                }
                std::thread::sleep(POLL_INTERVAL);
            }
        }
    }
}

impl OwnerState {
    /// Serve requests until ownership is lost or the connection dies.
    fn event_loop(self) {
        let mut incrs: HashMap<(Window, Atom), IncrTransfer> = HashMap::new();
        loop {
            let event = match self.conn.wait_for_event() {
                Ok(event) => event,
                Err(err) => {
                    tracing::warn!(error = %err, "x11 clipboard connection lost; offer retired");
                    return;
                }
            };
            match event {
                Event::SelectionRequest(req) => {
                    if let Err(err) = self.handle_request(&mut incrs, &req) {
                        tracing::warn!(error = %err, x11_target = req.target, "x11 clipboard request failed");
                    }
                }
                // Ownership lost to another client (or a superseding
                // serve() of our own): normal operation, exit quietly.
                Event::SelectionClear(_) => {
                    tracing::debug!("x11 clipboard ownership lost; serving thread exiting");
                    return;
                }
                Event::PropertyNotify(notify) if notify.state == Property::DELETE => {
                    if let Err(err) =
                        self.handle_incr_delete(&mut incrs, notify.window, notify.atom)
                    {
                        tracing::warn!(error = %err, "x11 INCR chunk write failed; transfer dropped");
                        incrs.remove(&(notify.window, notify.atom));
                    }
                }
                Event::Error(err) => {
                    tracing::warn!(error = ?err, "x11 protocol error during clipboard serving");
                }
                _ => {}
            }
            if let Err(err) = self.conn.flush() {
                tracing::warn!(error = %err, "x11 clipboard flush failed; offer retired");
                return;
            }
        }
    }

    /// Answer one `SelectionRequest` and send the `SelectionNotify`.
    fn handle_request(
        &self,
        incrs: &mut HashMap<(Window, Atom), IncrTransfer>,
        req: &SelectionRequestEvent,
    ) -> Result<(), ClipboardError> {
        // Legacy convention: property None means "store under the target".
        let property = if req.property == x11rb::NONE {
            req.target
        } else {
            req.property
        };
        let answered = if req.target == self.atoms.targets {
            self.write_targets(req.requestor, property)?;
            property
        } else if req.target == self.atoms.timestamp {
            change_property32(
                &self.conn,
                req.requestor,
                property,
                self.atoms.integer,
                &[self.time],
            )?;
            property
        } else if req.target == self.atoms.multiple {
            if self.handle_multiple(incrs, req, property)? {
                property
            } else {
                x11rb::NONE
            }
        } else if let Some(served) = self.served.targets.iter().find(|t| t.target == req.target) {
            self.write_data(incrs, req.requestor, property, served)?;
            property
        } else {
            x11rb::NONE
        };
        self.send_notify(req, answered)
    }

    /// `TARGETS`: protocol targets plus every servable target atom.
    fn write_targets(&self, requestor: Window, property: Atom) -> Result<(), ClipboardError> {
        let mut atoms = Vec::with_capacity(PROTOCOL_TARGETS.len() + self.served.targets.len());
        atoms.push(self.atoms.targets);
        atoms.push(self.atoms.timestamp);
        atoms.push(self.atoms.multiple);
        atoms.extend(self.served.targets.iter().map(|t| t.target));
        change_property32(&self.conn, requestor, property, self.atoms.atom, &atoms)
    }

    /// One data target: direct property write, or INCR when the payload
    /// exceeds the chunk budget.
    fn write_data(
        &self,
        incrs: &mut HashMap<(Window, Atom), IncrTransfer>,
        requestor: Window,
        property: Atom,
        served: &ServedTarget,
    ) -> Result<(), ClipboardError> {
        if served.data.len() <= self.chunk {
            return change_property8(
                &self.conn,
                requestor,
                property,
                served.prop_type,
                &served.data,
            );
        }
        // Watch the requestor's property so its Delete notifications
        // (one per consumed chunk) reach us.
        self.conn
            .change_window_attributes(
                requestor,
                &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
            )
            .map_err(ClipboardError::transport)?
            .check()
            .map_err(ClipboardError::transport)?;
        let total = u32::try_from(served.data.len())
            .map_err(|_| ClipboardError::X11("INCR payload exceeds u32".to_owned()))?;
        change_property32(&self.conn, requestor, property, self.atoms.incr, &[total])?;
        incrs.insert(
            (requestor, property),
            IncrTransfer {
                data: Arc::clone(&served.data),
                offset: 0,
                chunk: self.chunk,
                prop_type: served.prop_type,
            },
        );
        Ok(())
    }

    /// The requestor deleted the INCR property: write the next chunk, or
    /// the zero-length terminator once the payload is exhausted.
    fn handle_incr_delete(
        &self,
        incrs: &mut HashMap<(Window, Atom), IncrTransfer>,
        requestor: Window,
        property: Atom,
    ) -> Result<(), ClipboardError> {
        let key = (requestor, property);
        let Some(mut transfer) = incrs.remove(&key) else {
            return Ok(());
        };
        let (range, finished) =
            next_incr_slice(transfer.offset, transfer.chunk, transfer.data.len());
        transfer.offset = range.end;
        if !finished {
            incrs.insert(key, transfer.clone());
        }
        change_property8(
            &self.conn,
            requestor,
            property,
            transfer.prop_type,
            &transfer.data[range],
        )
    }

    /// `MULTIPLE` (ICCCM 2.6.2): read the `ATOM_PAIR` list from the
    /// requestor and convert each target into its own property; a target
    /// that cannot be converted gets its property deleted (type None).
    /// The returned bool reports whether the list itself was readable.
    fn handle_multiple(
        &self,
        incrs: &mut HashMap<(Window, Atom), IncrTransfer>,
        req: &SelectionRequestEvent,
        property: Atom,
    ) -> Result<bool, ClipboardError> {
        let reply = self
            .conn
            .get_property(
                false,
                req.requestor,
                property,
                self.atoms.atom_pair,
                0,
                MULTIPLE_LIST_LIMIT,
            )
            .map_err(ClipboardError::transport)?
            .reply()
            .map_err(ClipboardError::transport)?;
        if reply.type_ != self.atoms.atom_pair || reply.format != 32 {
            return Ok(false);
        }
        for (target, dest) in atom_pairs(&reply.value) {
            if dest == x11rb::NONE {
                continue;
            }
            let result = if target == self.atoms.targets {
                self.write_targets(req.requestor, dest)
            } else if target == self.atoms.timestamp {
                change_property32(
                    &self.conn,
                    req.requestor,
                    dest,
                    self.atoms.integer,
                    &[self.time],
                )
            } else if target == self.atoms.multiple {
                // Nested MULTIPLE is not allowed by ICCCM: refuse.
                Err(ClipboardError::X11("nested MULTIPLE refused".to_owned()))
            } else if let Some(served) = self.served.targets.iter().find(|t| t.target == target) {
                self.write_data(incrs, req.requestor, dest, served)
            } else {
                Err(ClipboardError::X11(
                    "unknown target in MULTIPLE list".to_owned(),
                ))
            };
            if result.is_err() {
                self.conn
                    .delete_property(req.requestor, dest)
                    .map_err(ClipboardError::transport)?
                    .check()
                    .map_err(ClipboardError::transport)?;
            }
        }
        Ok(true)
    }

    /// `SelectionNotify` back to the requestor (`property` = None
    /// refuses the conversion).
    fn send_notify(
        &self,
        req: &SelectionRequestEvent,
        property: Atom,
    ) -> Result<(), ClipboardError> {
        let notify = SelectionNotifyEvent {
            response_type: x11rb::protocol::xproto::SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: req.time,
            requestor: req.requestor,
            selection: req.selection,
            target: req.target,
            property,
        };
        self.conn
            .send_event(false, req.requestor, EventMask::NO_EVENT, notify)
            .map_err(ClipboardError::transport)?
            .check()
            .map_err(ClipboardError::transport)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The epoch tests touch the process-global [`OWNER_EPOCH`]; they are
    /// one sequential test body so no other test observes a mid-sequence
    /// epoch (nothing else in the headless suite claims one — `serve()`
    /// needs an X server).
    #[test]
    fn loss_report_fires_only_for_the_latest_ownership_claim() {
        // Given: a recording hook and a claimed epoch.
        let fired = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let hook: OfferLossHook = Arc::new({
            let fired = Arc::clone(&fired);
            move || fired.store(true, Ordering::Release)
        });
        let epoch = next_owner_epoch();
        // When: the thread exits while its claim is still the latest.
        report_loss_if_latest(Some(&hook), epoch);
        // Then: the loss is reported.
        assert!(fired.load(Ordering::Acquire));

        // Given: a superseding serve() claims the next epoch.
        fired.store(false, Ordering::Release);
        let newer = next_owner_epoch();
        // When: the superseded thread exits afterwards.
        report_loss_if_latest(Some(&hook), epoch);
        // Then: the self-supersede stays silent…
        assert!(!fired.load(Ordering::Acquire));
        // …and the latest claim still reports.
        report_loss_if_latest(Some(&hook), newer);
        assert!(fired.load(Ordering::Acquire));

        // No hook configured: the report is a silent no-op.
        report_loss_if_latest(None, newer);

        // Given: a live claim, then a newer serve() that fails its setup.
        fired.store(false, Ordering::Release);
        let live = next_owner_epoch();
        let failed = next_owner_epoch();
        // When: the failed setup unclaims its epoch.
        unclaim_owner_epoch(failed);
        // Then: the live claim's genuine loss still reports - the dead
        // claim no longer silences it.
        report_loss_if_latest(Some(&hook), live);
        assert!(fired.load(Ordering::Acquire));
        // And: a subsequent live claim resumes the normal silence rule.
        fired.store(false, Ordering::Release);
        let newest = next_owner_epoch();
        report_loss_if_latest(Some(&hook), live);
        assert!(!fired.load(Ordering::Acquire));
        report_loss_if_latest(Some(&hook), newest);
        assert!(fired.load(Ordering::Acquire));
    }
}
