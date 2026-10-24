//! Armarium's Library: database, Import, Metadata and Cover extraction.

pub mod azw3;
pub mod cover;
pub mod cover_search;
pub mod db;
pub mod device;
mod edit;
mod error;
pub mod extract;
#[cfg(test)]
mod fixtures;
pub mod import;
pub mod library;
pub mod merge;
pub mod model;
mod naming;
pub mod suggest;
pub mod text;
mod xml;

pub use error::{Error, Result};
pub use library::Library;
