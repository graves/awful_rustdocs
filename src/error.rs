use std::{fmt, io, path::PathBuf};
use serde;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    // config / fs
    ConfigDirUnavailable,
    Io { path: Option<PathBuf>, source: io::Error },

    // external tools
    ToolSpawn { tool: &'static str, source: io::Error },
    ToolWait  { tool: &'static str, source: io::Error },
    ToolStatus { tool: &'static str, code: Option<i32>, stderr_hint: Option<String> },

    // parsing / serde
    Json { context: &'static str, source: serde_json::Error },

    // integration points (foreign error types → string)
    External { context: &'static str, message: String },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use Error::*;
        match self {
            ConfigDirUnavailable => write!(f, "could not determine OS config directory"),
            Io { path, source } => match path {
                Some(p) => write!(f, "I/O error at {}: {}", p.display(), source),
                None => write!(f, "I/O error: {}", source),
            },
            ToolSpawn { tool, source } => write!(f, "failed to spawn {}: {}", tool, source),
            ToolWait { tool, source } => write!(f, "failed to wait on {}: {}", tool, source),
            ToolStatus { tool, code, .. } => write!(f, "{} exited with status {:?}", tool, code),
            Json { context, source } => write!(f, "JSON error in {}: {}", context, source),
            External { context, message } => write!(f, "{}: {}", context, message),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        use Error::*;
        match self {
            Io { source, .. } => Some(source),
            ToolSpawn { source, .. } => Some(source),
            ToolWait { source, .. } => Some(source),
            Json { source, .. } => Some(source),
            ToolStatus { .. } | ConfigDirUnavailable | External { .. } => None,
        }
    }
}
