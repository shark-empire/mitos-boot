pub mod recovery;

#[derive(Debug, thiserror::Error)]
pub enum BootError {
    #[error("configuration error: {0}")]  Config(String),
    #[error("display error: {0}")]        Display(String),
    #[error("renderer error: {0}")]       Renderer(String),
    #[error("video error: {0}")]          Video(String),
    #[error("asset error: {0}")]          Asset(String),
    #[error("ipc error: {0}")]            Ipc(String),
    #[error("system error: {0}")]         System(String),
    #[error("handoff error: {0}")]        Handoff(String),
    #[error(transparent)]                 Io(#[from] std::io::Error),
}

pub type BootResult<T> = Result<T, BootError>;