use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum DomainError {
    #[error("no conversion table for {from} -> {to}")]
    MissingTable { from: String, to: String },
    #[error("text is empty")]
    EmptyText,
    #[error("cannot detect layout direction of text")]
    UnknownDirection,
    #[error("nothing to convert")]
    NoInputFound,
    #[error("conversion disabled")]
    Disabled,
    #[error("invalid layout id: {0}")]
    InvalidLayout(String),
}
