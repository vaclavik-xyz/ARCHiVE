use super::{
    address::DetectedAddress, animation::Animation, currency::DetectedCurrency, style::Style,
    unit::Unit,
};

/// Text effect container
///
/// Message text may contain any number of traditional styles or one animation.
///
/// Read more about text styles [here](https://www.apple.com/newsroom/2024/06/ios-18-makes-iphone-more-personal-capable-and-intelligent-than-ever/).
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum TextEffect {
    /// Default, unstyled text
    Default,
    /// A [mentioned](https://support.apple.com/guide/messages/mention-a-person-icht306ee34b/mac) contact in the conversation
    ///
    /// The embedded data contains information about the mentioned contact.
    Mention(String),
    /// A clickable link, i.e. `https://`, `tel:`, `mailto:`, and others
    ///
    /// The embedded data contains the url.
    Link(String),
    /// A one-time code, i.e. from a 2FA message
    OTP,
    /// A detected postal address within the message text
    ///
    /// The embedded data contains the address components. It is boxed because
    /// [`DetectedAddress`] is large relative to the other variants, which would
    /// otherwise inflate every `TextEffect` to its size.
    Address(Box<DetectedAddress>),
    /// Traditional formatting styles
    ///
    /// The embedded data contains the formatting styles applied to the range.
    Styles(Vec<Style>),
    /// Animation applied to the text
    ///
    /// The embedded data contains the animation applied to the range.
    Animated(Animation),
    /// Conversions that can be applied to text
    ///
    /// The embedded data contains the unit that the range represents.
    Conversion(Unit),
    /// A detected monetary amount within the message text
    ///
    /// The embedded data contains the currency symbol and amount.
    Currency(DetectedCurrency),
}
