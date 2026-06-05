/// Animated text effect container
///
/// A message's [`typedstream`](crate::util::typedstream) contains an [`i64`] identifier under the key `__kIMTextEffectAttributeName`.
///
/// Read more about text styles [here](https://www.apple.com/newsroom/2024/06/ios-18-makes-iphone-more-personal-capable-and-intelligent-than-ever/).
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Animation {
    /// Denoted by an ID of `5`
    Big,
    /// Denoted by an ID of `11`
    Small,
    /// Denoted by an ID of `9`
    Shake,
    /// Denoted by an ID of `8`
    Nod,
    /// Denoted by an ID of `12`
    Explode,
    /// Denoted by an ID of `4`
    Ripple,
    /// Denoted by an ID of `6`
    Bloom,
    /// Denoted by an ID of `10`
    Jitter,
    /// A new identifier not currently supported
    Unknown(i64),
}

impl Animation {
    /// Get the animation from its ID given in a message's [`typedstream`](crate::util::typedstream) data, under the `__kIMTextEffectAttributeName` key.
    ///
    /// # Example:
    ///
    /// ```
    /// use imessage_database::message_types::text_effects::animation::Animation;
    ///
    /// let animation = Animation::from_id(5); // Animation::Big
    /// ```
    #[must_use]
    pub fn from_id(value: i64) -> Self {
        match value {
            // In order of appearance in the text effects menu
            5 => Self::Big,
            11 => Self::Small,
            9 => Self::Shake,
            8 => Self::Nod,
            12 => Self::Explode,
            4 => Self::Ripple,
            6 => Self::Bloom,
            10 => Self::Jitter,
            _ => Self::Unknown(value),
        }
    }
}
