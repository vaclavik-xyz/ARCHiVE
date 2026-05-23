// MARK: OptionalText

/// An `Option<&str>` that treats `Some("")` as absent.
///
/// iMessage plist payloads frequently carry empty-string fields where a
/// `None` would be more semantically accurate; wrapping balloon VM fields
/// in this type normalizes the two cases so templates don't emit blank
/// lines for empty-but-present strings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OptionalText<'a>(Option<&'a str>);

impl<'a> OptionalText<'a> {
    /// The wrapped value with empty strings filtered out.
    pub fn get(&self) -> Option<&'a str> {
        self.0
    }
}

impl<'a> From<Option<&'a str>> for OptionalText<'a> {
    fn from(value: Option<&'a str>) -> Self {
        Self(value.filter(|s| !s.is_empty()))
    }
}
