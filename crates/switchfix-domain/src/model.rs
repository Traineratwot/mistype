use serde::{Deserialize, Serialize};

/// Layout identifier matching KDE short name, e.g. `"us"`, `"ru"`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LayoutId(pub String);

impl LayoutId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for LayoutId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// 0-based index in KDE System Settings → Input Devices → Layouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LayoutIndex(pub u32);

impl LayoutIndex {
    pub fn as_u32(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutInfo {
    pub index: LayoutIndex,
    pub id: LayoutId,
    pub name: String,
}

/// Key chord for hotkeys and synthetic injection (e.g. Ctrl+Insert).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum KeyChord {
    CtrlC,
    CtrlShiftC,
    CtrlInsert,
    CtrlV,
    CtrlShiftV,
    CtrlA,
    CtrlK,
    CtrlF1,
    CtrlF2,
    CtrlShiftF1,
    CtrlPause,
    MetaSpace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CorrectionRequest {
    ConvertSelection,
    ConvertLastWord,
    ConvertTerminal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CorrectionOutcome {
    Converted {
        original: String,
        converted: String,
        layout_switched_to: Option<LayoutIndex>,
    },
    NoInputFound,
    Disabled,
    /// Text was replaced but layout switch failed (or similar partial success).
    Partial {
        original: String,
        converted: String,
        note: String,
    },
    Failed(String),
}

impl CorrectionOutcome {
    pub fn is_success(&self) -> bool {
        matches!(
            self,
            CorrectionOutcome::Converted { .. } | CorrectionOutcome::Partial { .. }
        )
    }
}
