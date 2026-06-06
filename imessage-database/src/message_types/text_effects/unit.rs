use plist::Value;

/// Unit conversion text effect container
///
/// Read more about unit conversions [here](https://www.macrumors.com/how-to/convert-currencies-temperatures-more-ios-16/).
///
/// The recognized unit identifiers are emitted verbatim by Apple's private
/// `DataDetectorsCore` framework. The full vocabulary was read from the
/// framework binary's `__cstring` section on macOS 26.5 (`arm64e`) via:
///
/// ```text
///   dyld_info -section __TEXT __cstring \
///     /System/Library/PrivateFrameworks/DataDetectorsCore.framework/DataDetectorsCore
/// ```
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Unit {
    /// Angle conversion
    Angle,
    /// Area conversion
    Area,
    /// Currency conversion
    Currency,
    /// Distance conversion
    Distance,
    /// Duration conversion
    Duration,
    /// Energy conversion
    Energy,
    /// Fuel efficiency conversion
    FuelEfficiency,
    /// Power conversion
    Power,
    /// Pressure conversion
    Pressure,
    /// Speed conversion
    Speed,
    /// Temperature conversion
    Temperature,
    /// Timezone conversion
    Timezone,
    /// Volume conversion
    Volume,
    /// Weight conversion
    Weight,
    /// A detected unit whose name did not map to any dimension above
    ///
    /// The embedded data is the raw unit name emitted by `DataDetectorsCore`,
    /// preserved verbatim.
    Unknown(String),
}

impl Unit {
    /// Parse a data-detector `NSKeyedArchiver` payload and extract the unit represented by the detector result.
    #[must_use]
    pub(crate) fn from_data_detected_plist(plist: &Value) -> Option<Self> {
        let body = plist.as_dictionary()?;
        let objects = body.get("$objects")?.as_array()?;
        let top = body.get("$top")?.as_dictionary()?;
        let root = top
            .get("dd-result")
            .or_else(|| top.get("root"))
            .and_then(Self::uid_index)?;

        Self::from_scanner_result(objects, root, 0)
    }

    fn from_scanner_result(objects: &[Value], idx: usize, depth: usize) -> Option<Self> {
        if depth > 8 {
            return None;
        }

        let result = objects.get(idx)?.as_dictionary()?;
        match Self::string_from_uid(objects, result.get("T")?)? {
            "Unit" => Self::from_unit_name(Self::string_from_uid(objects, result.get("V")?)?),
            "PhysicalAmount" => Self::child_results(objects, result.get("SR")?)?
                .iter()
                .filter_map(Self::uid_index)
                .find_map(|idx| Self::from_scanner_result(objects, idx, depth + 1)),
            _ => None,
        }
    }

    fn child_results<'a>(objects: &'a [Value], value: &Value) -> Option<&'a [Value]> {
        objects
            .get(Self::uid_index(value)?)?
            .as_dictionary()?
            .get("NS.objects")?
            .as_array()
            .map(Vec::as_slice)
    }

    fn string_from_uid<'a>(objects: &'a [Value], value: &Value) -> Option<&'a str> {
        objects.get(Self::uid_index(value)?)?.as_string()
    }

    fn uid_index(value: &Value) -> Option<usize> {
        Some(value.as_uid()?.get() as usize)
    }

    /// See [`Unit`] enum docs for source detail on internal strings.
    ///
    /// Currency and timezone conversions are detected through other channels
    /// (the scanner-result type and `__kIMCalendarEventAttributeName`), so they
    /// do not appear here. Imperial units arrive as `<base>-imperial`, so the
    /// suffix is stripped before matching; unrecognized names (including
    /// ambiguous composites) surface as [`Unit::Unknown`] carrying the raw,
    /// unstripped name.
    fn from_unit_name(name: &str) -> Option<Self> {
        match name.strip_suffix("-imperial").unwrap_or(name) {
            "celsius" | "fahrenheit" | "kelvin" => Some(Self::Temperature),
            "gram" | "kilogram" | "metric tonne" | "ounce" | "pound" | "stone" | "short ton" => {
                Some(Self::Weight)
            }
            "meter" | "kilometer" | "centimeter" | "millimeter" | "mile" | "yard" | "foot"
            | "inch" => Some(Self::Distance),
            "liter" | "centiliter" | "milliliter" | "cubic meter" | "cubic centimeter"
            | "cubic foot" | "cubic inch" | "gallon" | "pint" | "quart" | "cup" | "fluid ounce"
            | "tablespoon" | "teaspoon" => Some(Self::Volume),
            "watt" | "kilowatt" | "horse power" => Some(Self::Power),
            "kilometer per hour" | "mile per hour" | "meter per second" => Some(Self::Speed),
            "liter per 100 kilometers" | "mile per gallon" => Some(Self::FuelEfficiency),
            "joule" | "kilojoule" | "calorie" | "kilocalorie" | "kilowatt hour" => {
                Some(Self::Energy)
            }
            "hectare" | "acre" | "square kilometer" | "square meter" | "square centimeter"
            | "square mile" | "square yard" | "square foot" | "square inch" => Some(Self::Area),
            "degree" | "radian" | "turn" => Some(Self::Angle),
            "bar" | "kilopascal" | "hectopascal" | "millimeter of mercury" | "psi" => {
                Some(Self::Pressure)
            }
            "hour" | "minute" | "second" | "millisecond" => Some(Self::Duration),
            _ => Some(Self::Unknown(name.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{env::current_dir, fs::File};

    use plist::Value;

    use super::Unit;

    fn unit_property(name: &str) -> Value {
        let plist_path = current_dir()
            .unwrap()
            .as_path()
            .join("test_data/unit_properties")
            .join(name);
        let plist_data = File::open(plist_path).unwrap();
        Value::from_reader(plist_data).unwrap()
    }

    #[test]
    fn can_parse_data_detected_physical_units() {
        for (fixture, expected) in [
            ("Temperature.plist", Unit::Temperature),
            ("Volume.plist", Unit::Volume),
            ("Weight.plist", Unit::Weight),
        ] {
            let plist = unit_property(fixture);
            assert_eq!(Unit::from_data_detected_plist(&plist), Some(expected));
        }
    }

    #[test]
    fn ignores_data_detected_non_units() {
        let plist = unit_property("HttpURL.plist");
        assert_eq!(Unit::from_data_detected_plist(&plist), None);
    }

    #[test]
    fn maps_unit_name_to_dimension() {
        for (name, expected) in [
            ("fahrenheit", Unit::Temperature),
            ("short ton", Unit::Weight),
            ("centimeter", Unit::Distance),
            ("fluid ounce", Unit::Volume),
            ("gallon-imperial", Unit::Volume), // imperial suffix stripped
            ("watt", Unit::Power),
            ("mile per hour", Unit::Speed),
            ("mile per gallon", Unit::FuelEfficiency),
            ("joule", Unit::Energy),
            ("acre", Unit::Area),
            ("radian", Unit::Angle),
            ("psi", Unit::Pressure),
            ("minute", Unit::Duration),
        ] {
            assert_eq!(Unit::from_unit_name(name), Some(expected), "{name}");
        }
        assert_eq!(
            Unit::from_unit_name("not a unit"),
            Some(Unit::Unknown("not a unit".to_string()))
        );
        // The ambiguous degree glyph arrives as a composite name; it is
        // preserved verbatim rather than dropped.
        assert_eq!(
            Unit::from_unit_name("celsius-fahrenheit-degree"),
            Some(Unit::Unknown("celsius-fahrenheit-degree".to_string()))
        );
    }
}
