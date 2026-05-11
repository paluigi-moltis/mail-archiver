use whatlang::{detect, Info, Lang};

/// Detect the language of the given text.
/// Returns an ISO 639-1 code (e.g., "en", "de", "fr") or None if undetectable.
#[allow(dead_code)]
pub fn detect_language(text: &str) -> Option<String> {
    // Use first 2000 chars for performance
    let sample = &text[..text.len().min(2000)];
    let info: Info = detect(sample)?;
    let code = lang_to_iso(info.lang());
    Some(code)
}

#[allow(dead_code)]
fn lang_to_iso(lang: Lang) -> String {
    match lang {
        Lang::Eng => "en".to_string(),
        Lang::Deu => "de".to_string(),
        Lang::Fra => "fr".to_string(),
        Lang::Ita => "it".to_string(),
        Lang::Spa => "es".to_string(),
        Lang::Por => "pt".to_string(),
        Lang::Nld => "nl".to_string(),
        Lang::Rus => "ru".to_string(),
        Lang::Cmn => "zh".to_string(),
        Lang::Jpn => "ja".to_string(),
        Lang::Kor => "ko".to_string(),
        Lang::Ara => "ar".to_string(),
        Lang::Pol => "pl".to_string(),
        Lang::Swe => "sv".to_string(),
        Lang::Dan => "da".to_string(),
        Lang::Nob => "no".to_string(),
        Lang::Fin => "fi".to_string(),
        Lang::Tur => "tr".to_string(),
        Lang::Hin => "hi".to_string(),
        _ => "unknown".to_string(),
    }
}

/// Map ISO 639-1 code to a PostgreSQL text search configuration name.
/// Falls back to 'english' for unsupported languages.
#[allow(dead_code)]
pub fn lang_to_ts_config(lang_code: &str) -> &'static str {
    match lang_code {
        "en" => "english",
        "de" => "german",
        "fr" => "french",
        "it" => "italian",
        "es" => "spanish",
        "pt" => "portuguese",
        "nl" => "dutch",
        "ru" => "russian",
        "da" => "danish",
        "fi" => "finnish",
        "sv" => "swedish",
        "no" => "norwegian",
        "tr" => "turkish",
        _ => "english",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_english() {
        let text = "This is a sample English text for testing language detection.";
        let lang = detect_language(text).unwrap();
        assert_eq!(lang, "en");
    }

    #[test]
    fn test_detect_german() {
        let text = "Dies ist ein deutscher Text zum Testen der Spracherkennung.";
        let lang = detect_language(text).unwrap();
        assert_eq!(lang, "de");
    }

    #[test]
    fn test_lang_to_ts_config() {
        assert_eq!(lang_to_ts_config("en"), "english");
        assert_eq!(lang_to_ts_config("de"), "german");
        assert_eq!(lang_to_ts_config("xx"), "english");
    }
}
