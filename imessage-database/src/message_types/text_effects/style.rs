/// Traditional text effect container
///
/// Read more about text styles [here](https://www.apple.com/newsroom/2024/06/ios-18-makes-iphone-more-personal-capable-and-intelligent-than-ever/).
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Style {
    /// **Bold** styled text
    Bold,
    /// *Italic* styled text
    Italic,
    /// ~~Strikethrough~~ styled text
    Strikethrough,
    /// <u>Underline</u> styled text
    Underline,
}
