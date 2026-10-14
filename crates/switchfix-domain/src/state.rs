//! State machine for a single correction operation.

/// States of `CorrectionService` during a hotkey-triggered correction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorrectionState {
    Idle,
    Capturing,
    Converting,
    Replacing,
    SwitchingLayout,
    RestoringClipboard,
    Error { recoverable: bool },
}

impl CorrectionState {
    /// Whether a new hotkey may start a correction in this state.
    pub fn can_start(&self) -> bool {
        matches!(self, CorrectionState::Idle)
    }

    /// Terminal-or-not: Error is considered terminal for the operation
    /// but the service returns to Idle after logging.
    pub fn is_error(&self) -> bool {
        matches!(self, CorrectionState::Error { .. })
    }
}

/// Explicit transitions (useful for tests and debugging).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorrectionEvent {
    Hotkey,
    Captured,
    Converted,
    Replaced,
    LayoutSwitched,
    ClipboardRestored,
    NoInput,
    Failed { recoverable: bool },
    Reset,
}

impl CorrectionState {
    pub fn apply(self, ev: CorrectionEvent) -> CorrectionState {
        use CorrectionEvent::*;
        use CorrectionState::*;
        match (self, ev) {
            (Idle, Hotkey) => Capturing,
            (Capturing, Captured) => Converting,
            (Capturing, NoInput) => Idle,
            (Converting, Converted) => Replacing,
            (Converting, NoInput) => Idle,
            (Replacing, Replaced) => SwitchingLayout,
            (SwitchingLayout, LayoutSwitched) => RestoringClipboard,
            (SwitchingLayout, Failed { recoverable }) => Error { recoverable },
            (RestoringClipboard, ClipboardRestored) => Idle,
            (RestoringClipboard, Failed { recoverable }) => Error { recoverable },
            (Error { .. }, Reset) => Idle,
            // Allow abort from any active state.
            (_, NoInput) => Idle,
            (_, Failed { recoverable }) => Error { recoverable },
            (st, _) => st,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path() {
        let mut st = CorrectionState::Idle;
        assert!(st.can_start());
        st = st.apply(CorrectionEvent::Hotkey);
        assert_eq!(st, CorrectionState::Capturing);
        st = st.apply(CorrectionEvent::Captured);
        assert_eq!(st, CorrectionState::Converting);
        st = st.apply(CorrectionEvent::Converted);
        assert_eq!(st, CorrectionState::Replacing);
        st = st.apply(CorrectionEvent::Replaced);
        assert_eq!(st, CorrectionState::SwitchingLayout);
        st = st.apply(CorrectionEvent::LayoutSwitched);
        assert_eq!(st, CorrectionState::RestoringClipboard);
        st = st.apply(CorrectionEvent::ClipboardRestored);
        assert_eq!(st, CorrectionState::Idle);
        assert!(st.can_start());
    }

    #[test]
    fn no_input_returns_idle() {
        let st = CorrectionState::Idle
            .apply(CorrectionEvent::Hotkey)
            .apply(CorrectionEvent::NoInput);
        assert_eq!(st, CorrectionState::Idle);
    }

    #[test]
    fn layout_fail_is_recoverable_error() {
        let st = CorrectionState::Idle
            .apply(CorrectionEvent::Hotkey)
            .apply(CorrectionEvent::Captured)
            .apply(CorrectionEvent::Converted)
            .apply(CorrectionEvent::Replaced)
            .apply(CorrectionEvent::Failed { recoverable: true });
        assert_eq!(
            st,
            CorrectionState::Error {
                recoverable: true
            }
        );
        assert!(st.is_error());
        let st = st.apply(CorrectionEvent::Reset);
        assert!(st.can_start());
    }

    #[test]
    fn cannot_start_while_busy() {
        assert!(!CorrectionState::Capturing.can_start());
        assert!(!CorrectionState::Replacing.can_start());
    }
}
