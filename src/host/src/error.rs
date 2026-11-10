#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Database(rusqlite::Error),
    Json(serde_json::Error),
    Invalid(String),
}
pub type Result<T> = std::result::Result<T, Error>;
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => e.fmt(f),
            Self::Database(e) => e.fmt(f),
            Self::Json(e) => e.fmt(f),
            Self::Invalid(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Self::Database(e)
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}
impl From<&str> for Error {
    fn from(e: &str) -> Self {
        Self::Invalid(e.into())
    }
}
impl From<String> for Error {
    fn from(e: String) -> Self {
        Self::Invalid(e)
    }
}
