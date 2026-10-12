//! 扩展名 → FileKind 映射表（engine 文档 4.3.4）。
use crate::model::FileKind;

/// ext 为小写、不带点；不在表里的归 Other。
pub(crate) fn from_ext(ext: &str) -> FileKind {
    match ext {
        "doc" | "docx" | "odt" | "rtf" | "txt" | "md" | "wps" => FileKind::Document,
        "xls" | "xlsx" | "xlsm" | "csv" | "ods" | "et" => FileKind::Spreadsheet,
        "ppt" | "pptx" | "odp" | "key" | "dps" => FileKind::Presentation,
        "pdf" => FileKind::Pdf,
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "svg" | "ico" | "tif" | "tiff"
        | "heic" | "psd" | "ai" => FileKind::Image,
        "mp4" | "mkv" | "mov" | "avi" | "wmv" | "flv" | "webm" | "m4v" => FileKind::Video,
        "mp3" | "wav" | "flac" | "aac" | "m4a" | "ogg" | "wma" => FileKind::Audio,
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" | "iso" => FileKind::Archive,
        "exe" | "msi" | "msix" | "appx" | "appxbundle" => FileKind::Installer,
        "rs" | "c" | "cpp" | "h" | "hpp" | "cs" | "js" | "ts" | "jsx" | "tsx" | "py" | "java"
        | "go" | "kt" | "swift" | "json" | "toml" | "yaml" | "yml" | "xml" | "html" | "css"
        | "sql" | "sh" | "ps1" | "bat" => FileKind::Code,
        _ => FileKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds() {
        assert_eq!(from_ext("pdf"), FileKind::Pdf);
        assert_eq!(from_ext("docx"), FileKind::Document);
        assert_eq!(from_ext("xlsx"), FileKind::Spreadsheet);
        assert_eq!(from_ext("png"), FileKind::Image);
        assert_eq!(from_ext("mp4"), FileKind::Video);
        assert_eq!(from_ext("zip"), FileKind::Archive);
        assert_eq!(from_ext("exe"), FileKind::Installer);
        assert_eq!(from_ext("rs"), FileKind::Code);
        assert_eq!(from_ext(""), FileKind::Other);
        assert_eq!(from_ext("unknownext"), FileKind::Other);
    }
}
