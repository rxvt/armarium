//! Armarium's Library: database, Import, Metadata and Cover extraction.

mod cover;
pub mod db;
mod edit;
mod error;
pub mod extract;
#[cfg(test)]
mod fixtures;
pub mod import;
pub mod library;
pub mod model;
mod naming;
pub mod text;
mod xml;

pub use error::{Error, Result};
pub use library::Library;
