//! Pure domain logic for SwitchFix — no I/O, no OS crates.

pub mod buffer;
pub mod conversion;
pub mod error;
pub mod model;
pub mod spell;
pub mod state;

pub use buffer::{TypingBuffer, WordSpan};
pub use conversion::{
    convert, detect_direction, en_to_ru_map, ConversionSet, ConversionTable, ConvertedText, MapFile,
};
pub use error::DomainError;
pub use spell::{
    confusable_neighbors, detect_lang, edit1_neighbors, edit_distance, filter_candidates,
    is_auto_fixable, one_step_neighbors, preserve_case, rank, should_auto_apply, should_check,
    suggestion_confidence, Suggestion, SuggestionKind,
};
pub use model::{
    CorrectionOutcome, CorrectionRequest, KeyChord, LayoutId, LayoutIndex, LayoutInfo,
};
pub use state::{CorrectionEvent, CorrectionState};
