//! Armarium's Library: database, Import, Metadata and Cover extraction.

pub mod db;
mod error;
pub mod extract;
#[cfg(test)]
mod fixtures;
pub mod model;
pub mod text;
mod xml;

pub use error::{Error, Result};
