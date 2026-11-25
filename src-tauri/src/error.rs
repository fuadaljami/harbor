use serde::Serialize;
use thiserror::Error;

/// Convenience result alias using Harbor's typed [`AppError`].
pub type AppResult<T> = Result<T, AppError>;

/// Typed application error serialized as `{ kind, message }` to the frontend.
#[derive(Debug, Error)]
pub enum AppError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Configuration parse error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Item not found: {0}")]
    NotFound(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("Launch cancelled by user")]
    LaunchCancelled,

    #[error("Failed to launch target: {0}")]
    LaunchFailed(String),

    #[error("Windows shell error: {0}")]
    Shell(String),

    #[error("Window operation failed: {0}")]
    Window(String),

    #[error("Global shortcut error: {0}")]
    Hotkey(String),

    #[error("File dialog error: {0}")]
    Dialog(String),

    #[error("Background worker unavailable: {0}")]
    WorkerUnavailable(String),

    #[error("Runtime error: {0}")]
    Tauri(String),
}

impl AppError {
    /// Stable machine-readable error kind used by the frontend toast router.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Io(_) => "io",
            Self::Json(_) | Self::Config(_) => "config",
            Self::NotFound(_) => "not_found",
            Self::InvalidInput(_) => "invalid_input",
            Self::LaunchCancelled => "launch_cancelled",
            Self::LaunchFailed(_) => "launch_failed",
            Self::Shell(_) => "shell",
            Self::Window(_) => "window",
            Self::Hotkey(_) => "hotkey",
            Self::Dialog(_) => "dialog",
            Self::WorkerUnavailable(_) => "worker_unavailable",
            Self::Tauri(_) => "tauri",
        }
    }
}

impl From<tauri::Error> for AppError {
    fn from(err: tauri::Error) -> Self {
        Self::Tauri(err.to_string())
    }
}

#[derive(Serialize)]
struct SerializedAppError {
    kind: &'static str,
    message: String,
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        SerializedAppError {
            kind: self.kind(),
            message: self.to_string(),
        }
        .serialize(serializer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_error_as_kind_and_message() -> Result<(), Box<dyn std::error::Error>> {
        let err = AppError::LaunchCancelled;
        let json = serde_json::to_value(&err)?;
        assert_eq!(json["kind"], "launch_cancelled");
        assert_eq!(json["message"], "Launch cancelled by user");

        let io_err = AppError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "missing file",
        ));
        let io_json = serde_json::to_value(&io_err)?;
        assert_eq!(io_json["kind"], "io");
        let Some(message) = io_json["message"].as_str() else {
            panic!("message should be a string");
        };
        assert!(message.contains("missing file"));
        Ok(())
    }
}
