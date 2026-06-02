//! Plugin discovery: finding installed plugins on disk and, later, reading
//! their descriptors into a catalog.
//!
//! [`paths`] answers *where to look* — the standard per-format directories the
//! host scans. Subsequent steps walk those directories and probe each bundle's
//! factory for its descriptor.

pub mod paths;
