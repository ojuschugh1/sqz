use thiserror::Error;

/// Location in source for error reporting
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLocation {
    pub file: String,
    pub line: u32,
    pub column: u32,
}

impl std::fmt::Display for SourceLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}:{}", self.file, self.line, self.column)
    }
}

/// Top-level error type for sqz_engine
#[derive(Debug, Error)]
pub enum SqzError {
    #[error("compression error: {0}")]
    Compression(String),

    #[error("session store error: {0}")]
    SessionStore(#[from] rusqlite::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("TOML parse error: {0}")]
    TomlParse(#[from] toml::de::Error),

    #[error("TOML serialize error: {0}")]
    TomlSerialize(#[from] toml::ser::Error),

    #[error("parse error at {location}: {message}")]
    Parse {
        location: SourceLocation,
        message: String,
    },

    #[error("preset validation error: field `{field}` — {message}")]
    PresetValidation { field: String, message: String },

    #[error("cache error: {0}")]
    Cache(String),

    #[error("plugin error in `{plugin}`: {message}")]
    Plugin { plugin: String, message: String },

    #[error("unsupported language: {0}")]
    UnsupportedLanguage(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, SqzError>;
