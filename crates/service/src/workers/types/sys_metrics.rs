//! Host metrics sampler state for the `system_metrics` worker.

use sysinfo::{Disks, Networks, System};

/// Cached `sysinfo` handles refreshed each publish tick.
pub struct SystemMetricsSampler {
    pub system: System,
    pub disks: Disks,
    pub networks: Networks,
}
