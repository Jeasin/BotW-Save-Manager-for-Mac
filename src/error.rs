// SPDX-License-Identifier: AGPL-3.0-only

use std::fmt;
use std::io;
use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    Io {
        action: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    InvalidInput(String),
    UnsupportedSave(String),
    Output(String),
    Verification(String),
}

impl Error {
    pub fn io(action: &'static str, path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            action,
            path: path.into(),
            source,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io {
                action,
                path,
                source,
            } => write!(f, "failed to {action} '{}': {source}", path.display()),
            Self::InvalidInput(message) => write!(f, "invalid input: {message}"),
            Self::UnsupportedSave(message) => write!(f, "unsupported save: {message}"),
            Self::Output(message) => write!(f, "output error: {message}"),
            Self::Verification(message) => write!(f, "conversion verification failed: {message}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
