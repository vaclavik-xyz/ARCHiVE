/*!
 Recompress embedded PDF images from lossless to JPEG.

 The headless browser stores raster images in print output losslessly
 (FlateDecode RGB/Gray), which makes image-heavy conversations enormous —
 image bytes scale with pixel count, not visual quality. This pass walks a
 rendered PDF, re-encodes each lossless photographic image as JPEG (DCTDecode),
 and rewrites the stream in place. Images it can't safely convert (already JPEG,
 unusual bit depth, indexed/CMYK color, 1-bit masks, or pixel data that doesn't
 match the expected layout) are left untouched.
*/

use std::{collections::HashSet, fs::rename, path::Path, path::PathBuf};

use jpeg_encoder::{ColorType, Encoder};
use lopdf::{Document, Object, ObjectId};

/// Recompress every eligible lossless image in `path` to JPEG at `quality`,
/// saving atomically. Returns the number of images converted.
pub(super) fn recompress_images(path: &Path, quality: u8) -> Result<u64, String> {
    let mut doc =
        Document::load(path).map_err(|why| format!("could not load {}: {why}", path.display()))?;

    // Gather image object ids up front so we can mutate while iterating.
    let image_ids: Vec<ObjectId> = doc
        .objects
        .iter()
        .filter(|(_, object)| is_image_xobject(object))
        .map(|(id, _)| *id)
        .collect();

    // Soft masks (`/SMask`) are smooth grayscale alpha and recompress safely at
    // high quality; hard masks (`/Mask` references) key on exact values, so they
    // are left lossless.
    let (soft_mask_ids, hard_mask_ids) = collect_masks(&doc, &image_ids);
    let mask_quality = quality.max(88);

    let mut recompressed = 0u64;
    for id in image_ids {
        if hard_mask_ids.contains(&id) {
            continue;
        }
        let q = if soft_mask_ids.contains(&id) {
            mask_quality
        } else {
            quality
        };
        let Some(jpeg) = encode_as_jpeg(&doc, id, q) else {
            continue;
        };
        if let Ok(Object::Stream(stream)) = doc.get_object_mut(id) {
            stream.dict.set("Filter", Object::Name(b"DCTDecode".to_vec()));
            stream.dict.remove(b"DecodeParms");
            stream.dict.remove(b"DecodeParams");
            stream.dict.set("Length", jpeg.len() as i64);
            stream.content = jpeg;
            recompressed += 1;
        }
    }

    if recompressed > 0 {
        // Write to a sibling and rename, so a failed save never truncates the
        // already-valid rendered PDF.
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".rc.tmp");
        let tmp = PathBuf::from(tmp);
        doc.save(&tmp)
            .map_err(|why| format!("could not write {}: {why}", tmp.display()))?;
        rename(&tmp, path)
            .map_err(|why| format!("could not finalize {}: {why}", path.display()))?;
    }
    Ok(recompressed)
}

/// Partition mask object ids into soft masks (`/SMask`, recompressible at high
/// quality) and hard masks (`/Mask` references, left lossless).
fn collect_masks(
    doc: &Document,
    image_ids: &[ObjectId],
) -> (HashSet<ObjectId>, HashSet<ObjectId>) {
    let mut soft = HashSet::new();
    let mut hard = HashSet::new();
    for &id in image_ids {
        let Ok(Object::Stream(stream)) = doc.get_object(id) else {
            continue;
        };
        if let Ok(reference) = stream.dict.get(b"SMask").and_then(|o| o.as_reference()) {
            soft.insert(reference);
        }
        if let Ok(reference) = stream.dict.get(b"Mask").and_then(|o| o.as_reference()) {
            hard.insert(reference);
        }
    }
    (soft, hard)
}

/// Whether `object` is an image XObject stream.
fn is_image_xobject(object: &Object) -> bool {
    matches!(object, Object::Stream(stream)
        if stream.dict.get(b"Subtype").ok().and_then(|o| o.as_name().ok()) == Some(&b"Image"[..]))
}

/// Produce JPEG bytes for the lossless image `id`, or `None` when it should not
/// be touched.
fn encode_as_jpeg(doc: &Document, id: ObjectId, quality: u8) -> Option<Vec<u8>> {
    let Object::Stream(stream) = doc.get_object(id).ok()? else {
        return None;
    };
    let dict = &stream.dict;

    // Only lossless (Flate) images; never re-encode JPEG/JBIG2/etc.
    if !filter_is_flate(dict.get(b"Filter").ok()?) {
        return None;
    }
    // 1-bit stencil masks are not photographs.
    if dict
        .get(b"ImageMask")
        .ok()
        .and_then(|o| o.as_bool().ok())
        .unwrap_or(false)
    {
        return None;
    }
    // Color-key masking keys on exact pixel values; lossy JPEG would break it.
    if matches!(dict.get(b"Mask").ok(), Some(Object::Array(_))) {
        return None;
    }
    if dict.get(b"BitsPerComponent").ok()?.as_i64().ok()? != 8 {
        return None;
    }

    let width = dict.get(b"Width").ok()?.as_i64().ok()?;
    let height = dict.get(b"Height").ok()?.as_i64().ok()?;
    if width <= 0 || height <= 0 || width > u16::MAX as i64 || height > u16::MAX as i64 {
        return None;
    }

    let components = color_components(doc, dict.get(b"ColorSpace").ok()?)?;
    let color_type = match components {
        1 => ColorType::Luma,
        3 => ColorType::Rgb,
        _ => return None,
    };

    let pixels = stream.decompressed_content().ok()?;
    if pixels.len() != (width as usize) * (height as usize) * components {
        // A predictor or padding we don't model — leave it lossless.
        return None;
    }

    let mut out = Vec::new();
    Encoder::new(&mut out, quality)
        .encode(&pixels, width as u16, height as u16, color_type)
        .ok()?;
    Some(out)
}

/// Whether a `/Filter` entry is exactly `FlateDecode`.
fn filter_is_flate(filter: &Object) -> bool {
    match filter {
        Object::Name(name) => name == b"FlateDecode",
        Object::Array(items) => {
            items.len() == 1 && matches!(&items[0], Object::Name(name) if name == b"FlateDecode")
        }
        _ => false,
    }
}

/// Component count for a PDF color space, resolving references and `ICCBased`
/// `/N`. Returns `None` for spaces we won't recompress (indexed, CMYK, …).
fn color_components(doc: &Document, color_space: &Object) -> Option<usize> {
    match color_space {
        Object::Name(name) => match name.as_slice() {
            b"DeviceRGB" | b"CalRGB" | b"RGB" => Some(3),
            b"DeviceGray" | b"CalGray" | b"G" => Some(1),
            _ => None,
        },
        Object::Reference(reference) => color_components(doc, doc.get_object(*reference).ok()?),
        Object::Array(items) => {
            let head = items.first()?.as_name().ok()?;
            match head {
                b"ICCBased" => {
                    let icc = doc.get_object(items.get(1)?.as_reference().ok()?).ok()?;
                    match icc.as_stream().ok()?.dict.get(b"N").ok()?.as_i64().ok()? {
                        1 => Some(1),
                        3 => Some(3),
                        _ => None,
                    }
                }
                b"CalRGB" => Some(3),
                b"CalGray" => Some(1),
                _ => None,
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::recompress_images;
    use lopdf::{Document, Object, Stream, dictionary};

    #[test]
    fn recompresses_lossless_rgb_image_to_jpeg() {
        let dir = crate::app::test_dir::unique_test_dir("recompress");
        let path = dir.join("img.pdf");

        let (w, h) = (64usize, 48usize);
        let mut image = Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Width" => w as i64,
                "Height" => h as i64,
                "BitsPerComponent" => 8,
                "ColorSpace" => "DeviceRGB",
            },
            vec![128u8; w * h * 3],
        );
        image.compress().unwrap(); // store as FlateDecode

        let mut doc = Document::with_version("1.5");
        let image_id = doc.add_object(image);
        let pages_id = doc.new_object_id();
        let resources_id =
            doc.add_object(dictionary! { "XObject" => dictionary! { "Im0" => image_id } });
        let content_id = doc.add_object(Stream::new(dictionary! {}, b"q /Im0 Do Q".to_vec()));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 64.into(), 48.into()],
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
            }),
        );
        let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog_id);
        doc.save(&path).unwrap();

        let converted = recompress_images(&path, 70).expect("recompress runs");
        assert_eq!(converted, 1, "the single image is converted");

        let reloaded = Document::load(&path).expect("reloads");
        let stream = reloaded
            .get_object(image_id)
            .unwrap()
            .as_stream()
            .unwrap();
        assert_eq!(
            stream.dict.get(b"Filter").unwrap().as_name().unwrap(),
            b"DCTDecode",
            "image stored as JPEG now"
        );
        assert!(
            stream.content.len() < w * h * 3,
            "JPEG is smaller than raw RGB"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recompresses_base_image_and_soft_mask() {
        let dir = crate::app::test_dir::unique_test_dir("recompress-smask");
        let path = dir.join("smask.pdf");
        let (w, h) = (32usize, 32usize);

        // Grayscale soft mask.
        let mut mask = Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Image",
                "Width" => w as i64, "Height" => h as i64,
                "BitsPerComponent" => 8, "ColorSpace" => "DeviceGray",
            },
            vec![200u8; w * h],
        );
        mask.compress().unwrap();

        let mut doc = Document::with_version("1.5");
        let mask_id = doc.add_object(mask);

        let mut base = Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Image",
                "Width" => w as i64, "Height" => h as i64,
                "BitsPerComponent" => 8, "ColorSpace" => "DeviceRGB",
                "SMask" => mask_id,
            },
            vec![64u8; w * h * 3],
        );
        base.compress().unwrap();
        let base_id = doc.add_object(base);

        let pages_id = doc.new_object_id();
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 32.into(), 32.into()],
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
            }),
        );
        let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog_id);
        doc.save(&path).unwrap();

        // Both the base image and its soft mask are recompressed.
        assert_eq!(recompress_images(&path, 70).unwrap(), 2, "base image and soft mask");

        let reloaded = Document::load(&path).unwrap();
        let filter_of = |id| {
            reloaded
                .get_object(id)
                .unwrap()
                .as_stream()
                .unwrap()
                .dict
                .get(b"Filter")
                .unwrap()
                .as_name()
                .unwrap()
                .to_vec()
        };
        assert_eq!(filter_of(base_id), b"DCTDecode", "base image becomes JPEG");
        assert_eq!(filter_of(mask_id), b"DCTDecode", "soft mask becomes JPEG");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn skips_hard_mask_references() {
        let dir = crate::app::test_dir::unique_test_dir("recompress-hardmask");
        let path = dir.join("hardmask.pdf");
        let (w, h) = (16usize, 16usize);

        let mut hard = Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Image",
                "Width" => w as i64, "Height" => h as i64,
                "BitsPerComponent" => 8, "ColorSpace" => "DeviceGray",
            },
            vec![255u8; w * h],
        );
        hard.compress().unwrap();

        let mut doc = Document::with_version("1.5");
        let hard_id = doc.add_object(hard);
        let mut base = Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Image",
                "Width" => w as i64, "Height" => h as i64,
                "BitsPerComponent" => 8, "ColorSpace" => "DeviceRGB",
                "Mask" => hard_id,
            },
            vec![10u8; w * h * 3],
        );
        base.compress().unwrap();
        doc.add_object(base);
        let pages_id = doc.new_object_id();
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 16.into(), 16.into()],
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
            }),
        );
        let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog_id);
        doc.save(&path).unwrap();

        // The base recompresses; the hard mask it references is left lossless.
        assert_eq!(recompress_images(&path, 70).unwrap(), 1, "only the base image");
        let reloaded = Document::load(&path).unwrap();
        assert_eq!(
            reloaded.get_object(hard_id).unwrap().as_stream().unwrap()
                .dict.get(b"Filter").unwrap().as_name().unwrap(),
            b"FlateDecode",
            "hard mask stays lossless"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn leaves_non_flate_images_untouched() {
        let dir = crate::app::test_dir::unique_test_dir("recompress-skip");
        let path = dir.join("noimg.pdf");
        // A document with no images: nothing to recompress.
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 10.into(), 10.into()],
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
            }),
        );
        let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog_id);
        doc.save(&path).unwrap();

        assert_eq!(recompress_images(&path, 70).unwrap(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
