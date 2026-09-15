//! Remote control from the user's own phone.
//!
//! Toglet has no inbound listener: it polls a bridge the user runs. The bridge is untrusted;
//! commands are authenticated end to end by [`mac`] with a secret shared by both ends.

pub mod envelope;
pub mod guard;
pub mod mac;
pub mod poll;
pub mod store;
