/// Why a toolbox could not be built, or a tool not run.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Package(String),
    #[error("{0}")]
    Engine(String),
    #[error("no such tool: {0}")]
    UnknownTool(String),
    #[error("{0}")]
    InvalidArguments(String),
    #[error("{0}")]
    Failed(String),
}
