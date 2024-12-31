use serde::{Deserialize, Serialize};
use tokio::task::spawn_blocking;

// Need to gate this under `experimental` feature flag.
#[doc(hidden)]
pub fn current_exe_name() -> std::io::Result<String> {
    Ok(std::env::current_exe()?
        .file_name()
        .expect("this should not happen here")
        .to_string_lossy()
        .into())
}

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq, Hash)]
pub struct ProcEnv {
    pub proc_id: u32,
    pub proc_name: Option<String>,
}

impl ProcEnv {
    // Need to gate this under `proc-env` & `sysinfo` & `wgpu` feature flag.
    pub fn create() -> Self {
        let proc_id = std::process::id();
        let proc_name = current_exe_name().ok();

        Self { proc_id, proc_name }
    }

    pub async fn create_async() -> Option<Self> {
        spawn_blocking(Self::create).await.ok()
    }
}
