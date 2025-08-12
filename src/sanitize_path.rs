use log::warn;
use std::io;

const TRIM_CHARS: &[char] = &['\0', ' ', '\n', '\t', '\r', '/', '.'];
const END_OF_STRING_CHARS: &[char] = &['\0', '\n', '\r'];

pub fn sanitize_path(path: &str) -> Result<String, io::Error> {
    let sanitized_path = path.trim_matches(TRIM_CHARS).replace('\\', "/");

    if let Some(idx) = sanitized_path.rfind('/') {
        let (dir_part, _) = sanitized_path.split_at(idx);

        // Check for ".." only in the directory part
        if dir_part.contains("..") {
            warn!("path «{path}» contains .. in directory part, this isn't supported");
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Path contains invalid '..' in directory part",
            ));
        }
    }

    match sanitized_path.find(END_OF_STRING_CHARS) {
        Some(idx) => {
            let (final_path, _) = sanitized_path.split_at(idx);
            Ok(final_path.to_string())
        }
        None => Ok(sanitized_path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_path() {
        // Normal filename
        assert_eq!(sanitize_path("filename.ext").unwrap(), "filename.ext");

        // Normal path
        assert_eq!(
            sanitize_path("folder\\file.ext").unwrap(),
            "folder/file.ext"
        );

        // Unix path
        assert_eq!(sanitize_path("folder/file.ext").unwrap(), "folder/file.ext");

        // Any number or ../ at the start will be removed.
        assert_eq!(
            sanitize_path("../folder/file.ext").unwrap(),
            "folder/file.ext"
        );

        // .. anywhere in the dir part will error out.
        assert!(sanitize_path("folder/../file.ext").is_err());

        // new line/empty chars at the end should be removed
        assert_eq!(
            sanitize_path("folder/file.ext\r\n\0").unwrap(),
            "folder/file.ext"
        );

        // anything after a new line should be trimmed off
        assert_eq!(
            sanitize_path("folder/file.ext\n00").unwrap(),
            "folder/file.ext"
        );
    }
}
