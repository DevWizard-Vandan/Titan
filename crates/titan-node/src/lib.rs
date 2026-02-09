//! Titan Node - Production orchestration layer.
//!
//! Spawns and coordinates:
//! - Engine Thread (CPU-pinned, hot path)
//! - Network Thread (TCP gateway)
//! - Metrics Thread (Prometheus exporter)
//! - Snapshot Thread (background persistence)

pub mod metrics;
pub mod snapshot;
