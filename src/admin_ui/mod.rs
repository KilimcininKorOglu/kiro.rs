//! Admin UI static file service module
//!
//! Uses rust-embed to embed frontend build artifacts

mod router;

pub use router::create_admin_ui_router;
