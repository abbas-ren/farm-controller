use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
pub struct WorkerManager {
    pub cancellation: CancellationToken,
    pub handles: Vec<JoinHandle<()>>,
}
