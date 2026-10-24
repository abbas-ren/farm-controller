//! Join-handle bundle returned by [`crate::workers::WorkerManager::start`].

use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Owns the shared cancellation token and spawned worker tasks for graceful shutdown.
pub struct WorkerManager {
    pub cancellation: CancellationToken,
    pub handles: Vec<JoinHandle<()>>,
}
