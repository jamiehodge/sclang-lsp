//! A language server for SuperCollider with no runtime dependency on sclang.
//!
//! Everything is answered from parsed source. Nothing in this crate spawns
//! sclang, connects to one, or requires one to exist — see ARCHITECTURE.md for
//! why running code is the editor's job rather than the language server's.

pub mod analysis;
pub mod documents;
pub mod features;
pub mod line_index;
pub mod locations;
pub mod references;
pub mod scope;
pub mod server;
pub mod workspace;
