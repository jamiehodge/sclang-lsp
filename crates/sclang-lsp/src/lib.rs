//! A language server for SuperCollider with no runtime dependency on sclang.
//!
//! The layering follows ARCHITECTURE.md: [`workspace`] and [`analysis`] are
//! tier 1, answering from parsed source alone. There is deliberately no tier 2
//! here yet — nothing in this crate talks to a running image.

pub mod analysis;
pub mod documents;
pub mod features;
pub mod line_index;
pub mod locations;
pub mod references;
pub mod scope;
pub mod server;
pub mod workspace;
