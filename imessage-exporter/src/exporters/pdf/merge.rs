/*!
 Merge several single-document PDFs into one.

 Each conversation is rendered in chunks (see [`super`]); this stitches the
 chunk PDFs back into a single document. Adapted from the canonical `lopdf`
 merge procedure: load every input, renumber objects so ids never collide,
 collect all pages under one shared `Pages` tree, and point a single `Catalog`
 at it.
*/

use std::{collections::BTreeMap, path::Path};

use lopdf::{Document, Object, ObjectId};

/// Merge `inputs` (in order) into a single PDF written to `output`.
pub(super) fn merge_pdfs(inputs: &[std::path::PathBuf], output: &Path) -> Result<(), String> {
    if inputs.is_empty() {
        return Err("no PDF chunks to merge".to_string());
    }

    let mut max_id = 1;
    // `page_order` preserves page sequence across inputs; `documents_pages`
    // maps each page id to its object. Page object ids are not guaranteed to be
    // monotonic, so the ordered vector — not the map's key order — drives `Kids`.
    let mut page_order: Vec<ObjectId> = Vec::new();
    let mut documents_pages: BTreeMap<ObjectId, Object> = BTreeMap::new();
    let mut documents_objects: BTreeMap<ObjectId, Object> = BTreeMap::new();
    let mut document = Document::with_version("1.5");

    for path in inputs {
        let mut doc =
            Document::load(path).map_err(|why| format!("could not load {}: {why}", path.display()))?;
        doc.renumber_objects_with(max_id);
        max_id = doc.max_id + 1;

        // `get_pages` is keyed by page number, so iteration is in page order.
        for (_, object_id) in doc.get_pages() {
            if let Ok(object) = doc.get_object(object_id) {
                page_order.push(object_id);
                documents_pages.insert(object_id, object.to_owned());
            }
        }
        documents_objects.extend(doc.objects);
    }

    // Locate (and merge) the Catalog and Pages dictionaries across inputs.
    let mut catalog_object: Option<(ObjectId, Object)> = None;
    let mut pages_object: Option<(ObjectId, Object)> = None;

    for (object_id, object) in &documents_objects {
        let type_name = object.as_dict().ok().and_then(|d| d.get(b"Type").ok()).and_then(|t| t.as_name().ok());
        match type_name {
            Some(b"Catalog") => {
                let id = catalog_object.as_ref().map_or(*object_id, |(id, _)| *id);
                catalog_object = Some((id, object.clone()));
            }
            Some(b"Pages") => {
                if let Ok(dict) = object.as_dict() {
                    let mut dict = dict.clone();
                    if let Some((_, older)) = pages_object.as_ref() {
                        if let Ok(older_dict) = older.as_dict() {
                            dict.extend(older_dict);
                        }
                    }
                    let id = pages_object.as_ref().map_or(*object_id, |(id, _)| *id);
                    pages_object = Some((id, Object::Dictionary(dict)));
                }
            }
            // Page nodes and outline trees are rebuilt below / dropped.
            Some(b"Page") | Some(b"Outlines") | Some(b"Outline") => {}
            _ => {
                document.objects.insert(*object_id, object.clone());
            }
        }
    }

    let Some((pages_id, pages_obj)) = pages_object else {
        return Err("merged chunks have no Pages tree".to_string());
    };
    let Some((catalog_id, catalog_obj)) = catalog_object else {
        return Err("merged chunks have no Catalog".to_string());
    };

    // Re-parent every page onto the shared Pages node.
    for (object_id, object) in &documents_pages {
        if let Ok(dict) = object.as_dict() {
            let mut dict = dict.clone();
            dict.set("Parent", pages_id);
            document.objects.insert(*object_id, Object::Dictionary(dict));
        }
    }

    // Rebuild the Pages node with all collected pages as its kids, in page
    // order (not object-id order).
    if let Ok(dict) = pages_obj.as_dict() {
        let mut dict = dict.clone();
        dict.set("Count", page_order.len() as u32);
        dict.set(
            "Kids",
            page_order
                .iter()
                .map(|id| Object::Reference(*id))
                .collect::<Vec<_>>(),
        );
        document.objects.insert(pages_id, Object::Dictionary(dict));
    }

    // Point the Catalog at the shared Pages node, dropping stale outlines.
    if let Ok(dict) = catalog_obj.as_dict() {
        let mut dict = dict.clone();
        dict.set("Pages", pages_id);
        dict.remove(b"Outlines");
        document.objects.insert(catalog_id, Object::Dictionary(dict));
    }

    document.trailer.set("Root", catalog_id);
    document.max_id = document.objects.len() as u32;
    document.renumber_objects();
    // Collapse the duplicate images/masks/fonts each chunk re-embedded.
    dedup_streams(&mut document);
    document.compress();
    document
        .save(output)
        .map_err(|why| format!("could not write {}: {why}", output.display()))?;
    Ok(())
}

/// Collapse byte-identical streams (duplicate images, masks, and font subsets
/// that the per-chunk renders each embedded) into a single shared object,
/// rewriting every reference to the survivor. Reply quotes and repeated stickers
/// can otherwise embed the same image many times across chunks.
fn dedup_streams(document: &mut Document) {
    use std::collections::HashMap;
    use std::hash::{Hash, Hasher};

    // Hash buckets group candidates; equality is then confirmed exactly, so a
    // hash collision can never merge two genuinely different streams.
    let mut buckets: HashMap<u64, Vec<ObjectId>> = HashMap::new();
    let mut remap: HashMap<ObjectId, ObjectId> = HashMap::new();

    // Deterministic order so the surviving object is stable.
    let mut stream_ids: Vec<ObjectId> = document
        .objects
        .iter()
        .filter(|(_, object)| matches!(object, Object::Stream(_)))
        .map(|(id, _)| *id)
        .collect();
    stream_ids.sort();

    for id in stream_ids {
        let digest = {
            let Some(Object::Stream(stream)) = document.objects.get(&id) else {
                continue;
            };
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            stream.content.hash(&mut hasher);
            // Hash the whole dictionary (except the byte-length bookkeeping) so
            // any field that changes interpretation participates.
            let mut keys: Vec<Vec<u8>> = stream.dict.iter().map(|(k, _)| k.clone()).collect();
            keys.sort();
            for key in &keys {
                if key.as_slice() == b"Length" {
                    continue;
                }
                if let Ok(value) = stream.dict.get(key) {
                    key.hash(&mut hasher);
                    format!("{value:?}").hash(&mut hasher);
                }
            }
            hasher.finish()
        };

        let bucket = buckets.entry(digest).or_default();
        let survivor = bucket
            .iter()
            .copied()
            .find(|&keeper| streams_equivalent(document, id, keeper));
        match survivor {
            Some(keeper) => {
                remap.insert(id, keeper);
            }
            None => bucket.push(id),
        }
    }

    if remap.is_empty() {
        return;
    }

    for (_, object) in document.objects.iter_mut() {
        rewrite_references(object, &remap);
    }
    for duplicate in remap.keys() {
        document.objects.remove(duplicate);
    }
}

/// Whether two stream objects are interchangeable: identical content and
/// identical dictionaries apart from the `/Length` bookkeeping entry.
fn streams_equivalent(document: &Document, a: ObjectId, b: ObjectId) -> bool {
    let (Some(Object::Stream(sa)), Some(Object::Stream(sb))) =
        (document.objects.get(&a), document.objects.get(&b))
    else {
        return false;
    };
    if sa.content != sb.content {
        return false;
    }
    let mut da = sa.dict.clone();
    let mut db = sb.dict.clone();
    da.remove(b"Length");
    db.remove(b"Length");
    da == db
}

/// Recursively repoint references in `object` from duplicate ids to survivors.
fn rewrite_references(
    object: &mut Object,
    remap: &std::collections::HashMap<ObjectId, ObjectId>,
) {
    match object {
        Object::Reference(id) => {
            if let Some(&keeper) = remap.get(id) {
                *id = keeper;
            }
        }
        Object::Array(items) => items.iter_mut().for_each(|o| rewrite_references(o, remap)),
        Object::Dictionary(dict) => {
            dict.iter_mut().for_each(|(_, o)| rewrite_references(o, remap));
        }
        Object::Stream(stream) => {
            stream
                .dict
                .iter_mut()
                .for_each(|(_, o)| rewrite_references(o, remap));
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::merge_pdfs;
    use lopdf::content::{Content, Operation};
    use lopdf::{Document, Object, Stream, dictionary};
    use std::path::Path;

    /// Build a minimal one-page PDF containing `text`.
    fn make_pdf(path: &Path, text: &str) {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let resources_id = doc.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        });
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 24.into()]),
                Operation::new("Td", vec![100.into(), 600.into()]),
                Operation::new("Tj", vec![Object::string_literal(text)]),
                Operation::new("ET", vec![]),
            ],
        };
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        });
        let pages = dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        };
        doc.objects.insert(pages_id, Object::Dictionary(pages));
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog", "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog_id);
        doc.save(path).unwrap();
    }

    #[test]
    fn merges_pdfs_preserving_page_order_and_content() {
        let dir = crate::app::test_dir::unique_test_dir("merge-pdfs");
        let a = dir.join("a.pdf");
        let b = dir.join("b.pdf");
        let c = dir.join("c.pdf");
        let out = dir.join("out.pdf");
        make_pdf(&a, "Alpha");
        make_pdf(&b, "Bravo");
        make_pdf(&c, "Charlie");

        merge_pdfs(&[a, b, c], &out).expect("merge succeeds");

        let merged = Document::load(&out).expect("merged PDF loads");
        let pages: Vec<_> = merged.get_pages().into_values().collect();
        assert_eq!(pages.len(), 3, "all pages survive the merge");

        // Page content must appear in input order, not be dropped or shuffled.
        let texts: Vec<String> = pages
            .iter()
            .map(|&page_id| {
                let content = merged.get_page_content(page_id).expect("page content");
                String::from_utf8_lossy(&content).into_owned()
            })
            .collect();
        assert!(texts[0].contains("Alpha"), "first page is Alpha");
        assert!(texts[1].contains("Bravo"), "second page is Bravo");
        assert!(texts[2].contains("Charlie"), "third page is Charlie");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Build a one-page PDF embedding a fixed 16x16 RGB image.
    fn make_image_pdf(path: &Path) {
        use lopdf::Stream;
        let (w, h) = (16i64, 16i64);
        let mut image = Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Image",
                "Width" => w, "Height" => h,
                "BitsPerComponent" => 8, "ColorSpace" => "DeviceRGB",
            },
            vec![77u8; (w * h * 3) as usize],
        );
        image.compress().unwrap();

        let mut doc = Document::with_version("1.5");
        let image_id = doc.add_object(image);
        let resources_id =
            doc.add_object(dictionary! { "XObject" => dictionary! { "Im0" => image_id } });
        let content_id = doc.add_object(Stream::new(dictionary! {}, b"q /Im0 Do Q".to_vec()));
        let pages_id = doc.new_object_id();
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id, "Contents" => content_id,
            "Resources" => resources_id,
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
        doc.save(path).unwrap();
    }

    #[test]
    fn merge_deduplicates_identical_images() {
        let dir = crate::app::test_dir::unique_test_dir("merge-dedup");
        let a = dir.join("a.pdf");
        let b = dir.join("b.pdf");
        let out = dir.join("out.pdf");
        make_image_pdf(&a);
        make_image_pdf(&b); // identical image bytes

        merge_pdfs(&[a, b], &out).expect("merge succeeds");

        let merged = Document::load(&out).unwrap();
        let images = merged
            .objects
            .values()
            .filter(|o| {
                matches!(o, Object::Stream(s)
                    if s.dict.get(b"Subtype").ok().and_then(|v| v.as_name().ok()) == Some(&b"Image"[..]))
            })
            .count();
        assert_eq!(images, 1, "identical images across inputs collapse to one");
        assert_eq!(merged.get_pages().len(), 2, "both pages survive");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn merge_rejects_empty_input() {
        let dir = crate::app::test_dir::unique_test_dir("merge-empty");
        assert!(merge_pdfs(&[], &dir.join("out.pdf")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
