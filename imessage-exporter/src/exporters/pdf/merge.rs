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
