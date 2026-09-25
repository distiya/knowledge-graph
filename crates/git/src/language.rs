use ckg_langs::detect_language_from_path;

pub fn detect_language(path: &str) -> Option<&'static str> {
    if let Some(lang) = detect_language_from_path(path) {
        return Some(lang);
    }
    let detected = linguist::detect_language_by_extension(path).ok()?;
    for candidate in detected {
        if let Some(id) = ckg_langs::detect_language_from_name(candidate.name) {
            return Some(id);
        }
    }
    None
}
