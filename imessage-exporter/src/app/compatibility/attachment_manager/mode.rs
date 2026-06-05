use std::fmt::Display;

// MARK: Mode
/// Represents different ways the app can interact with attachment data
#[derive(Debug, PartialEq, Eq, Default)]
pub enum AttachmentManagerMode {
    /// Do not copy attachments
    #[default]
    Disabled,
    /// Copy and convert image attachments to more compatible formats using a [`Converter`](crate::app::compatibility::models::Converter)
    Basic,
    /// Copy attachments without converting; preserves quality but may not display correctly in all browsers
    Clone,
    /// Copy and convert all attachments to more compatible formats using a [`Converter`](crate::app::compatibility::models::Converter)
    Full,
}

impl AttachmentManagerMode {
    /// Create an instance of the enum given user input
    pub fn from_cli(copy_state: &str) -> Option<Self> {
        match copy_state.to_lowercase().as_str() {
            "disabled" => Some(Self::Disabled),
            "basic" => Some(Self::Basic),
            "clone" => Some(Self::Clone),
            "full" => Some(Self::Full),
            _ => None,
        }
    }
}

impl Display for AttachmentManagerMode {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AttachmentManagerMode::Disabled => write!(fmt, "disabled"),
            AttachmentManagerMode::Basic => write!(fmt, "basic"),
            AttachmentManagerMode::Clone => write!(fmt, "clone"),
            AttachmentManagerMode::Full => write!(fmt, "full"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AttachmentManagerMode;

    #[test]
    fn test_attachment_manager_mode() {
        assert_eq!(
            AttachmentManagerMode::from_cli("disabled"),
            Some(AttachmentManagerMode::Disabled)
        );
        assert_eq!(
            AttachmentManagerMode::from_cli("basic"),
            Some(AttachmentManagerMode::Basic)
        );
        assert_eq!(
            AttachmentManagerMode::from_cli("clone"),
            Some(AttachmentManagerMode::Clone)
        );
        assert_eq!(
            AttachmentManagerMode::from_cli("full"),
            Some(AttachmentManagerMode::Full)
        );
        assert_eq!(AttachmentManagerMode::from_cli("invalid"), None);
    }
}
