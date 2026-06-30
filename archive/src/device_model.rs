//! Translate Apple hardware model identifiers (e.g. `iPad4,1`) to their
//! marketing names (e.g. `iPad Air`) for human-facing reports. The machine
//! contract keeps the raw identifier; only HTML presentation is prettified.
//! Unknown identifiers fall back to themselves, so the table can lag new
//! hardware without losing information.

/// Marketing name for a hardware model identifier, or `None` when unknown.
pub fn marketing_name(model: &str) -> Option<&'static str> {
    let name = match model {
        // iPhone
        "iPhone3,1" | "iPhone3,2" | "iPhone3,3" => "iPhone 4",
        "iPhone4,1" => "iPhone 4S",
        "iPhone5,1" | "iPhone5,2" => "iPhone 5",
        "iPhone5,3" | "iPhone5,4" => "iPhone 5c",
        "iPhone6,1" | "iPhone6,2" => "iPhone 5s",
        "iPhone7,2" => "iPhone 6",
        "iPhone7,1" => "iPhone 6 Plus",
        "iPhone8,1" => "iPhone 6s",
        "iPhone8,2" => "iPhone 6s Plus",
        "iPhone8,4" => "iPhone SE (1. generace)",
        "iPhone9,1" | "iPhone9,3" => "iPhone 7",
        "iPhone9,2" | "iPhone9,4" => "iPhone 7 Plus",
        "iPhone10,1" | "iPhone10,4" => "iPhone 8",
        "iPhone10,2" | "iPhone10,5" => "iPhone 8 Plus",
        "iPhone10,3" | "iPhone10,6" => "iPhone X",
        "iPhone11,2" => "iPhone XS",
        "iPhone11,4" | "iPhone11,6" => "iPhone XS Max",
        "iPhone11,8" => "iPhone XR",
        "iPhone12,1" => "iPhone 11",
        "iPhone12,3" => "iPhone 11 Pro",
        "iPhone12,5" => "iPhone 11 Pro Max",
        "iPhone12,8" => "iPhone SE (2. generace)",
        "iPhone13,1" => "iPhone 12 mini",
        "iPhone13,2" => "iPhone 12",
        "iPhone13,3" => "iPhone 12 Pro",
        "iPhone13,4" => "iPhone 12 Pro Max",
        "iPhone14,4" => "iPhone 13 mini",
        "iPhone14,5" => "iPhone 13",
        "iPhone14,2" => "iPhone 13 Pro",
        "iPhone14,3" => "iPhone 13 Pro Max",
        "iPhone14,6" => "iPhone SE (3. generace)",
        "iPhone14,7" => "iPhone 14",
        "iPhone14,8" => "iPhone 14 Plus",
        "iPhone15,2" => "iPhone 14 Pro",
        "iPhone15,3" => "iPhone 14 Pro Max",
        "iPhone15,4" => "iPhone 15",
        "iPhone15,5" => "iPhone 15 Plus",
        "iPhone16,1" => "iPhone 15 Pro",
        "iPhone16,2" => "iPhone 15 Pro Max",
        "iPhone17,3" => "iPhone 16",
        "iPhone17,4" => "iPhone 16 Plus",
        "iPhone17,1" => "iPhone 16 Pro",
        "iPhone17,2" => "iPhone 16 Pro Max",

        // iPad
        "iPad1,1" => "iPad",
        "iPad2,1" | "iPad2,2" | "iPad2,3" | "iPad2,4" => "iPad 2",
        "iPad2,5" | "iPad2,6" | "iPad2,7" => "iPad mini",
        "iPad3,1" | "iPad3,2" | "iPad3,3" => "iPad (3. generace)",
        "iPad3,4" | "iPad3,5" | "iPad3,6" => "iPad (4. generace)",
        "iPad4,1" | "iPad4,2" | "iPad4,3" => "iPad Air",
        "iPad4,4" | "iPad4,5" | "iPad4,6" => "iPad mini 2",
        "iPad4,7" | "iPad4,8" | "iPad4,9" => "iPad mini 3",
        "iPad5,1" | "iPad5,2" => "iPad mini 4",
        "iPad5,3" | "iPad5,4" => "iPad Air 2",
        "iPad6,3" | "iPad6,4" => "iPad Pro (9,7\")",
        "iPad6,7" | "iPad6,8" => "iPad Pro (12,9\")",
        "iPad6,11" | "iPad6,12" => "iPad (5. generace)",
        "iPad7,1" | "iPad7,2" => "iPad Pro (12,9\", 2. generace)",
        "iPad7,3" | "iPad7,4" => "iPad Pro (10,5\")",
        "iPad7,5" | "iPad7,6" => "iPad (6. generace)",
        "iPad7,11" | "iPad7,12" => "iPad (7. generace)",
        "iPad8,1" | "iPad8,2" | "iPad8,3" | "iPad8,4" => "iPad Pro (11\")",
        "iPad8,5" | "iPad8,6" | "iPad8,7" | "iPad8,8" => "iPad Pro (12,9\", 3. generace)",
        "iPad8,9" | "iPad8,10" => "iPad Pro (11\", 2. generace)",
        "iPad8,11" | "iPad8,12" => "iPad Pro (12,9\", 4. generace)",
        "iPad11,1" | "iPad11,2" => "iPad mini (5. generace)",
        "iPad11,3" | "iPad11,4" => "iPad Air (3. generace)",
        "iPad11,6" | "iPad11,7" => "iPad (8. generace)",
        "iPad12,1" | "iPad12,2" => "iPad (9. generace)",
        "iPad13,1" | "iPad13,2" => "iPad Air (4. generace)",
        "iPad13,4" | "iPad13,5" | "iPad13,6" | "iPad13,7" => "iPad Pro (11\", 3. generace)",
        "iPad13,8" | "iPad13,9" | "iPad13,10" | "iPad13,11" => "iPad Pro (12,9\", 5. generace)",
        "iPad13,16" | "iPad13,17" => "iPad Air (5. generace)",
        "iPad13,18" | "iPad13,19" => "iPad (10. generace)",
        "iPad14,1" | "iPad14,2" => "iPad mini (6. generace)",

        // iPod touch
        "iPod5,1" => "iPod touch (5. generace)",
        "iPod7,1" => "iPod touch (6. generace)",
        "iPod9,1" => "iPod touch (7. generace)",

        _ => return None,
    };
    Some(name)
}

/// Human label for a model identifier: "iPad Air (iPad4,1)" when known, else the
/// bare identifier so an unmapped device still reads sensibly.
pub fn display_model(model: &str) -> String {
    match marketing_name(model) {
        Some(name) => format!("{name} ({model})"),
        None => model.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_known_identifiers() {
        assert_eq!(marketing_name("iPad4,1"), Some("iPad Air"));
        assert_eq!(marketing_name("iPhone8,1"), Some("iPhone 6s"));
        assert_eq!(marketing_name("iPhone13,2"), Some("iPhone 12"));
        assert_eq!(marketing_name("iPod7,1"), Some("iPod touch (6. generace)"));
    }

    #[test]
    fn unknown_identifier_is_none() {
        assert_eq!(marketing_name("iPad99,9"), None);
        assert_eq!(marketing_name(""), None);
    }

    #[test]
    fn display_model_appends_identifier_when_known() {
        assert_eq!(display_model("iPad4,1"), "iPad Air (iPad4,1)");
    }

    #[test]
    fn display_model_falls_back_to_bare_identifier() {
        assert_eq!(display_model("iPad99,9"), "iPad99,9");
        assert_eq!(display_model(""), "");
    }
}
