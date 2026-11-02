//! Owned messages crossing WebView callbacks and login workers into Tao's loop.
//!
//! Callbacks enqueue events; only the event-loop thread mutates `App`. Keep local
//! UI commands and remote firmware messages distinct even if their JSON `kind`
//! values happen to overlap.

use crate::connection::Outcome;

/// Identifies an embedded local UI, assigned by Rust rather than the JS payload.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Surface {
    Toolbar,
    Manager,
}

/// Do not derive `Debug`: payloads can contain credentials or session tokens.
///
/// `attempt` correlates worker results with a pending login; `session` correlates
/// firmware messages with the displayed viewer. They serve different lifetimes.
pub(crate) enum AppEvent {
    Local { surface: Surface, message: String },
    Remote { session: u64, message: String },
    Connection { attempt: u64, outcome: Outcome },
}
