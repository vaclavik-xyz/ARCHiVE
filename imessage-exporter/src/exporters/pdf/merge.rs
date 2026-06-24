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
    let mut documents_pages: BTreeMap<ObjectId, Object> = BTreeMap::new();
    let mut documents_objects: BTreeMap<ObjectId, Object> = BTreeMap::new();
    let mut document = Document::with_version("1.5");

    for path in inputs {
        let mut doc =
            Document::load(path).map_err(|why| format!("could not load {}: {why}", path.display()))?;
        doc.renumber_objects_with(max_id);
        max_id = doc.max_id + 1;

        for (_, object_id) in doc.get_pages() {
            if let Ok(object) = doc.get_object(object_id) {
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

    // Rebuild the Pages node with all collected pages as its kids.
    if let Ok(dict) = pages_obj.as_dict() {
        let mut dict = dict.clone();
        dict.set("Count", documents_pages.len() as u32);
        dict.set(
            "Kids",
            documents_pages
                .keys()
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
    document.compress();
    document
        .save(output)
        .map_err(|why| format!("could not write {}: {why}", output.display()))?;
    Ok(())
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
    fn merges_pdfs_preserving_all_pages() {
        let dir = std::env::temp_dir().join(format!("ime-merge-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.pdf");
        let b = dir.join("b.pdf");
        let c = dir.join("c.pdf");
        let out = dir.join("out.pdf");
        make_pdf(&a, "Alpha");
        make_pdf(&b, "Bravo");
        make_pdf(&c, "Charlie");

        merge_pdfs(&[a, b, c], &out).expect("merge succeeds");

        let merged = Document::load(&out).expect("merged PDF loads");
        assert_eq!(merged.get_pages().len(), 3, "all pages survive the merge");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn merge_rejects_empty_input() {
        let out = std::env::temp_dir().join("ime-merge-empty.pdf");
        assert!(merge_pdfs(&[], &out).is_err());
    }
}
