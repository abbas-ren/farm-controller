use sysinfo::{Disks, Networks, System};

pub struct SystemMetricsSampler {
    pub system: System,
    pub disks: Disks,
    pub networks: Networks,
}
