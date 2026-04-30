use mime_guess::MimeGuess;

fn is_generic_content_type(content_type: &str) -> bool {
    content_type.is_empty()
        || matches!(
            content_type,
            "application/octet-stream" | "binary/octet-stream"
        )
}

pub fn normalize_preview_content_type(path: &str, content_type: &str) -> String {
    let normalized = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if !is_generic_content_type(&normalized) {
        return normalized;
    }
    MimeGuess::from_path(path)
        .first_raw()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            if normalized.is_empty() {
                "application/octet-stream".to_string()
            } else {
                normalized
            }
        })
}

pub fn workspace_preview_supported(path: &str, content_type: &str) -> bool {
    let lowered_path = path.to_ascii_lowercase();
    let content_type = normalize_preview_content_type(path, content_type);
    content_type.starts_with("text/")
        || matches!(
            content_type.as_str(),
            "application/json"
                | "application/pdf"
                | "application/msword"
                | "application/rtf"
                | "application/xml"
                | "application/x-yaml"
                | "application/vnd.ms-excel"
                | "application/vnd.ms-powerpoint"
                | "application/vnd.openxmlformats-officedocument.presentationml.presentation"
                | "application/vnd.openxmlformats-officedocument.presentationml.slideshow"
                | "application/vnd.openxmlformats-officedocument.presentationml.template"
                | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
                | "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                | "image/svg+xml"
        )
        || content_type.starts_with("image/")
        || content_type.starts_with("audio/")
        || content_type.starts_with("video/")
        || content_type.starts_with("application/vnd.ms-excel.")
        || content_type.starts_with("application/vnd.ms-powerpoint.")
        || content_type.starts_with("application/vnd.ms-word.")
        || lowered_path.ends_with(".csv")
        || lowered_path.ends_with(".json")
        || lowered_path.ends_with(".md")
        || lowered_path.ends_with(".markdown")
        || lowered_path.ends_with(".txt")
        || lowered_path.ends_with(".html")
        || lowered_path.ends_with(".htm")
        || lowered_path.ends_with(".svg")
        || lowered_path.ends_with(".doc")
        || lowered_path.ends_with(".docm")
        || lowered_path.ends_with(".docx")
        || lowered_path.ends_with(".rtf")
        || lowered_path.ends_with(".xls")
        || lowered_path.ends_with(".xlsm")
        || lowered_path.ends_with(".xlsx")
        || lowered_path.ends_with(".ppt")
        || lowered_path.ends_with(".pptm")
        || lowered_path.ends_with(".pptx")
        || lowered_path.ends_with(".png")
        || lowered_path.ends_with(".jpg")
        || lowered_path.ends_with(".jpeg")
        || lowered_path.ends_with(".gif")
        || lowered_path.ends_with(".webp")
        || lowered_path.ends_with(".avif")
        || lowered_path.ends_with(".bmp")
        || lowered_path.ends_with(".ico")
        || lowered_path.ends_with(".mp3")
        || lowered_path.ends_with(".m4a")
        || lowered_path.ends_with(".ogg")
        || lowered_path.ends_with(".wav")
        || lowered_path.ends_with(".mp4")
        || lowered_path.ends_with(".mov")
        || lowered_path.ends_with(".webm")
}
