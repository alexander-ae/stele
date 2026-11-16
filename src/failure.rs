//! Why a command failed, for the shell.
//!
//! The message on stderr is for a person; the exit code is for a script. Only
//! two failures deserve a code of their own: the server is not there (try
//! again later) and the server said no (the same call will keep saying no).
//! Everything else is a local problem and stays `1`.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Name resolution, connect or read failed; no HTTP answer was read.
    Unreachable,
    /// An HTTP answer arrived and it was not a success.
    Refused,
}

impl Kind {
    pub fn exit_code(self) -> i32 {
        match self {
            Kind::Unreachable => 2,
            Kind::Refused => 3,
        }
    }
}

/// An error that knows which exit code it deserves. It travels inside
/// `anyhow` like any other, and `exit_code` digs it out of the context chain.
#[derive(Debug)]
pub struct Failure {
    kind: Kind,
    detail: String,
}

impl Failure {
    pub fn unreachable(detail: impl Into<String>) -> Self {
        Self {
            kind: Kind::Unreachable,
            detail: detail.into(),
        }
    }

    pub fn refused(detail: impl Into<String>) -> Self {
        Self {
            kind: Kind::Refused,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for Failure {}

/// The kind of the first `Failure` in the chain, if there is one.
pub fn kind(error: &anyhow::Error) -> Option<Kind> {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<Failure>())
        .map(|failure| failure.kind)
}

/// `0` success is the caller's business; this only maps an error.
pub fn exit_code(error: &anyhow::Error) -> i32 {
    kind(error).map(Kind::exit_code).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;

    #[test]
    fn a_plain_error_is_a_local_failure() {
        let error = anyhow::anyhow!("no such file");
        assert_eq!(kind(&error), None);
        assert_eq!(exit_code(&error), 1);
    }

    #[test]
    fn the_kind_survives_anyhow_context() {
        let error: anyhow::Error = Failure::unreachable("cannot connect").into();
        let error = Err::<(), _>(error)
            .with_context(|| "talking to somewhere")
            .unwrap_err();
        assert_eq!(kind(&error), Some(Kind::Unreachable));
        assert_eq!(exit_code(&error), 2);

        let error: anyhow::Error = Failure::refused("HTTP 401").into();
        let error = Err::<(), _>(error)
            .context("message 42 stays unread")
            .unwrap_err();
        assert_eq!(exit_code(&error), 3);
    }
}
