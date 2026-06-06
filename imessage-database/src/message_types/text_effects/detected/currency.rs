use crate::util::data_detected::{FromScannerResult, ScannerResult};

/// A detected monetary amount within message text.
///
/// Apple's `DataDetectorsCore` framework tags currency mentions (e.g. `$16`,
/// `$10k`, `$199.98`, or `5 dollars`) under the `__kIMMoneyAttributeName`
/// attribute as a `Money` [`ScannerResult`]. The nested `Currency` result
/// carries the symbol and the `Money` result's matched text carries the full
/// amount, so both fields are read straight from the payload without
/// re-parsing the message text.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct DetectedCurrency {
    /// The currency symbol exactly as it appeared, e.g. `$` or `dollars`.
    pub symbol: String,
    /// The amount exactly as it appeared, with the symbol removed, e.g. `16`,
    /// `10k`, `199.98`, or `275 million`.
    ///
    /// The amount is preserved verbatim rather than normalized: a thousands or
    /// millions multiplier (`k`, `million`) and any fractional portion stay in
    /// the string, since `DataDetectorsCore` does not store a single resolved
    /// numeric value.
    pub amount: String,
}

impl FromScannerResult for DetectedCurrency {
    /// Parse a currency from a `Money` scanner result.
    ///
    /// The symbol is the matched text of the nested `Currency` result; the
    /// amount is the `Money` result's matched text with that symbol removed.
    fn from_scanner_result(result: &ScannerResult<'_>) -> Option<Self> {
        if result.kind()? != "Money" {
            return None;
        }
        let matched = result.matched()?;
        let symbol = result
            .children()
            .find(|child| child.kind() == Some("Currency"))
            .and_then(|child| child.matched())?;
        Some(Self {
            amount: Self::amount_without_symbol(matched, symbol),
            symbol: symbol.to_string(),
        })
    }
}

impl DetectedCurrency {
    /// Recover the bare amount by stripping the currency `symbol` from the full
    /// matched text.
    ///
    /// The symbol always appears as either a prefix (`$16`) or a suffix
    /// (`5 dollars`) of the match, so removing it and trimming surrounding
    /// whitespace yields the amount. A symbol that is neither (not observed in
    /// practice) leaves the matched text intact rather than dropping it.
    fn amount_without_symbol(matched: &str, symbol: &str) -> String {
        matched
            .strip_prefix(symbol)
            .or_else(|| matched.strip_suffix(symbol))
            .unwrap_or(matched)
            .trim()
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::DetectedCurrency;

    #[test]
    fn amount_strips_prefix_symbol() {
        for (matched, symbol, expected) in [
            ("$16", "$", "16"),
            ("$199.98", "$", "199.98"), // fractional part preserved
            ("$10k", "$", "10k"),       // multiplier preserved
            ("$275 million", "$", "275 million"),
        ] {
            assert_eq!(
                DetectedCurrency::amount_without_symbol(matched, symbol),
                expected,
                "{matched}"
            );
        }
    }

    #[test]
    fn amount_strips_suffix_symbol() {
        assert_eq!(
            DetectedCurrency::amount_without_symbol("5 dollars", "dollars"),
            "5"
        );
    }

    #[test]
    fn amount_unstrippable_symbol_returns_match() {
        // Defensive: a symbol that is neither prefix nor suffix leaves the
        // matched text intact rather than dropping it.
        assert_eq!(DetectedCurrency::amount_without_symbol("16", "$"), "16");
    }
}
