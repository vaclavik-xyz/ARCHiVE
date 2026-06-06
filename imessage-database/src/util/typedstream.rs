/*!
 Helpers for working with `Property` types in the Crabstep deserializer.
*/

use crabstep::{OutputData, PropertyIterator, deserializer::iter::Property};

/// Represents a range pair that contains a type index and a length.
#[derive(Debug)]
pub struct TypeLengthPair {
    /// The type index of the property.
    pub type_index: i64,
    /// The length of the text affected by the referenced property.
    pub length: u64,
}

// MARK: Type Length
/// Converts a `Property` to a range pair used to denote a type index and a length.
#[inline(always)]
pub fn as_type_length_pair(property: &Property<'_, '_>) -> Option<TypeLengthPair> {
    if let Property::Group(group) = property {
        let mut iter = group.iter();
        if let Some(Property::Primitive(OutputData::SignedInteger(type_index))) = iter.next()
            && let Some(Property::Primitive(OutputData::UnsignedInteger(length))) = iter.next()
        {
            return Some(TypeLengthPair {
                type_index: *type_index,
                length: *length,
            });
        }
    }
    None
}

// MARK: i64
/// Converts a `Property` to an `Option<i64>` if it is a signed integer or similar structure.
#[must_use]
#[inline(always)]
pub fn as_signed_integer(property: &Property<'_, '_>) -> Option<i64> {
    if let Property::Group(group) = property {
        let val = group.iter().next()?;
        if let Property::Primitive(OutputData::SignedInteger(value)) = val {
            return Some(*value);
        } else if let Property::Object { name, mut data, .. } = val
            && name == "NSNumber"
        {
            return as_signed_integer(&data.next()?);
        }
    }
    None
}

// MARK: u64
/// Converts a `Property` to an `Option<u64>` if it is an unsigned integer or similar structure.
#[must_use]
#[inline(always)]
pub fn as_unsigned_integer(property: &Property<'_, '_>) -> Option<u64> {
    if let Property::Group(group) = property {
        let val = group.iter().next()?;
        if let Property::Primitive(OutputData::UnsignedInteger(value)) = val {
            return Some(*value);
        } else if let Property::Object { name, mut data, .. } = val
            && name == "NSNumber"
        {
            return as_unsigned_integer(&data.next()?);
        }
    }
    None
}

// MARK: f64
/// Converts a `Property` to an `Option<f64>` if it is a double or similar numeric structure.
#[must_use]
#[inline(always)]
pub fn as_float(property: &Property<'_, '_>) -> Option<f64> {
    if let Property::Group(group) = property {
        let val = group.iter().next()?;
        if let Property::Primitive(OutputData::Double(value)) = val {
            return Some(*value);
        } else if let Property::Object { name, mut data, .. } = val
            && name == "NSNumber"
        {
            return as_float(&data.next()?);
        }
    }
    None
}

// MARK: String
/// Converts a `Property` to an `Option<&str>` if it is a `NSString` or similar structure.
#[inline(always)]
pub fn as_nsstring<'a>(property: &Property<'a, 'a>) -> Option<&'a str> {
    if let Property::Group(group) = property
        && let Some(Property::Object { name, mut data, .. }) = group.iter().next()
        && (name == "NSString" || name == "NSAttributedString" || name == "NSMutableString")
        && let Some(Property::Group(prim)) = data.next()
        && let Some(Property::Primitive(OutputData::String(s))) = prim.first()
    {
        return Some(s);
    }
    None
}

// MARK: Data
/// Converts a `Property` to an `Option<&[u8]>` if it is `NSData` or its mutable
/// subclass `NSMutableData`.
#[inline(always)]
pub fn as_nsdata<'a>(property: &Property<'a, 'a>) -> Option<&'a [u8]> {
    if let Property::Group(group) = property
        && let Some(Property::Object { name, data, .. }) = group.iter().next()
        && (name == "NSData" || name == "NSMutableData")
    {
        for item in data {
            if let Property::Group(group) = item {
                for child in group {
                    if let Property::Primitive(OutputData::Array(bytes)) = child {
                        return Some(bytes);
                    }
                }
            }
        }
    }

    None
}

// MARK: Dict
/// Converts a `Property` to a `PropertyIterator` if it is a `NSDictionary`.
///
/// Returns the iterator **by value** (the lazy group yields owned properties).
#[inline(always)]
pub fn as_ns_dictionary<'a>(property: &Property<'a, 'a>) -> Option<PropertyIterator<'a, 'a>> {
    if let Property::Group(group) = property
        && let Some(Property::Object { name, data, .. }) = group.iter().next()
        && name == "NSDictionary"
    {
        return Some(data);
    }
    None
}

// MARK: NSURL
/// Given a resolved `Property`, walks two levels of nested groups under an
/// `NSURL`→`NSString` and returns the inner `&str`.
#[inline(always)]
pub fn as_nsurl<'a>(property: &Property<'a, 'a>) -> Option<&'a str> {
    // Only care about the top-level group.
    if let Property::Group(groups) = property {
        for level1 in groups.iter() {
            // Look for Object(name = "NSURL", data = ...).
            if let Property::Object {
                name,
                data: url_data,
                ..
            } = level1
                && name == "NSURL"
            {
                // First level under NSURL.
                for level2 in url_data {
                    if let Property::Group(inner) = level2 {
                        for level3 in inner.iter() {
                            // Look for Object(name = "NSString", data = ...).
                            if let Property::Object {
                                name,
                                data: str_data,
                                ..
                            } = level3
                                && name == "NSString"
                            {
                                // Second level under NSString.
                                for level4 in str_data {
                                    if let Property::Group(bottom) = level4
                                        && let Some(Property::Primitive(OutputData::String(s))) =
                                            bottom.first()
                                    {
                                        return Some(s);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}
