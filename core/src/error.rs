use std::fmt;
use std::io;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Database(rusqlite::Error),
    /// A Format file that cannot be read as its type, with a short reason
    /// suitable for the Import summary.
    Unreadable(String),
    /// A request that cannot be carried out, such as an empty title or an
    /// unknown Book.
    Invalid(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::Database(e) => write!(f, "database error: {e}"),
            Error::Unreadable(reason) | Error::Invalid(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::Database(e) => Some(e),
            Error::Unreadable(_) | Error::Invalid(_) => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Error {
        Error::Io(e)
    }
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Error {
        Error::Database(e)
    }
}
