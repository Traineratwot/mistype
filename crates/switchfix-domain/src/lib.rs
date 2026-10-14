//! Pure domain logic for SwitchFix — no I/O, no OS crates.

pub mod buffer;
pub mod conversion;
pub mod error;
pub mod model;
pub mod state;

pub use buffer::{TypingBuffer, WordSpan};
pub use conversion::{
    convert, detect_direction, en_to_ru_map, ConversionSet, ConversionTable, ConvertedText,
};
pub use error::DomainError;
pub use model::{
    CorrectionOutcome, CorrectionRequest, KeyChord, LayoutId, LayoutIndex, LayoutInfo,
};
pub use state::{CorrectionEvent, CorrectionState};
