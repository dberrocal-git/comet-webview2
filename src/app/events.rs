use crate::connection::Outcome;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Surface {
    Toolbar,
    Manager,
}

// Do not derive Debug: event payloads can contain credentials or session tokens.
pub(crate) enum AppEvent {
    Local { surface: Surface, message: String },
    Remote { session: u64, message: String },
    Connection { attempt: u64, outcome: Outcome },
}
