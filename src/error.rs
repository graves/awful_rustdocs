use std::{fmt, io, path::PathBuf};

pub type Result<T> = std::result::Result<T, Error>;

#[allow(unused)]
#[derive(Debug)]
pub enum Error {
    // config / fs
    ConfigDirUnavailable,
    Io {
        path: Option<PathBuf>,
        source: io::Error,
    },

    // external tools
    ToolSpawn {
        tool: &'static str,
        source: io::Error,
    },
    ToolWait {
        tool: &'static str,
        source: io::Error,
    },
    ToolStatus {
        tool: &'static str,
        code: Option<i32>,
        stderr_hint: Option<String>,
    },

    // parsing / serde
    Json {
        context: &'static str,
        source: serde_json::Error,
    },

    // integration points (foreign error types → string)
    External {
        context: &'static str,
        message: String,
    },
}

impl fmt::Display for Error {
    /// Formats the `Error` enum into a human-readable string for display purposes.
    ///
    /// This function converts an `Error` variant into a formatted error message that includes
    /// contextual details such as file paths, tool names, exit codes, or JSON contexts.
    /// The output is suitable for logging or user-facing error messages.
    ///
    /// Parameters:
    /// - `f`: A formatter into which the error message is written.
    ///
    /// Returns:
    /// - `fmt::Result`: The result of formatting the error message.
    ///
    /// Errors:
    /// - This function does not return errors; it formats the error and writes to the formatter.
    ///
    /// Notes:
    /// - The formatting varies by error type, providing context-specific messages.
    /// - For example, `Io` errors include the path and source, while `ToolSpawn` includes the tool name and source.
    /// - `External` errors include a context and a message, useful for external system failures.
    ///
    /// Examples:
    /// ```no_run
    /// use std::fmt;
    /// use crate::error::Error;
    /// let err = Error::Io { path: Some("/path/to/file".into()), source: "File not found".into() };
    /// let mut f = std::fmt::Write::new(String::new());
    /// fmt(&err, &mut f).unwrap();
    /// assert_eq!(f.to_string(), "I/O error at /path/to/file: File not found");
    /// ```
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
    /// Returns an optional reference to the underlying error source, if available.
    ///
    /// This function examines the error variant and returns a reference to the inner error
    /// source if the error is one of `Io`, `ToolSpawn`, `ToolWait`, or `Json`. For errors
    /// like `ToolStatus`, `ConfigDirUnavailable`, or `External`, no source is available
    /// and `None` is returned.
    ///
    /// # Returns
    /// - `Some(&dyn std::error::Error + 'static)` if the error has a source.
    /// - `None` if the error does not have a source or is a terminal error.
    ///
    /// # Errors
    /// - This function does not propagate errors; it only returns a reference to the source.
    /// - Errors are not returned directly; the caller must handle the `Option`.
    ///
    /// # Notes
    /// - The returned reference is borrowed from the internal error state and is valid for the
    ///   lifetime of the error.
    /// - This function is useful for propagating detailed error information in error chains.
    ///
    /// # Examples
    /// ```no_run
    /// use crate::error::Error;
    /// let err = Error::Io { source: Box::new(std::io::Error::new(std::io::ErrorKind::NotFound, "file not found")) };
    /// assert_eq!(err.source(), Some(&std::io::Error { .. }));
    /// let err = Error::ToolStatus {};
    /// assert_eq!(err.source(), None);
    /// ```
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
