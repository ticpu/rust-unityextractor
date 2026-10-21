//! Turns a `pathname` entry into a relative path that stays inside the
//! working directory.
//!
//! The path is handled per component, never per character: dots are part of
//! names (`.gitignore`, `file.`) and only the `.` and `..` components are
//! special. Leading `..` components are dropped so a package built with
//! relative paths still extracts in place; a `..` after a real component is
//! refused, since it could climb out of the working directory.

use log::warn;
use std::io;

const TRIM_CHARS: &[char] = &['\0', ' ', '\n', '\t', '\r'];
const END_OF_STRING_CHARS: &[char] = &['\0', '\n', '\r'];

pub fn sanitize_path(path: &str) -> Result<String, io::Error> {
    let trimmed = path.trim_matches(TRIM_CHARS);
    let line = match trimmed.find(END_OF_STRING_CHARS) {
        Some(idx) => &trimmed[..idx],
        None => trimmed,
    };
    let unified = line.replace('\\', "/");

    let mut components = Vec::new();
    for component in unified.split('/') {
        match component {
            "" | "." => {}
            ".." if components.is_empty() => {}
            ".." => {
                warn!("path «{path}» contains .. after its first component, this isn't supported");
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Path contains invalid '..' component",
                ));
            }
            name => components.push(name),
        }
    }
    Ok(components.join("/"))
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

        // .. after a real component will error out.
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

    #[test]
    fn test_dots_in_names_are_kept() {
        assert_eq!(sanitize_path(".gitignore").unwrap(), ".gitignore");
        assert_eq!(
            sanitize_path(".github/workflows/ci.yml").unwrap(),
            ".github/workflows/ci.yml"
        );
        assert_eq!(sanitize_path("Assets/file.").unwrap(), "Assets/file.");
        assert_eq!(
            sanitize_path("Assets/fo..lder/file.ext").unwrap(),
            "Assets/fo..lder/file.ext"
        );
    }

    #[test]
    fn test_dot_components_are_removed() {
        assert_eq!(
            sanitize_path("./Assets/file.ext").unwrap(),
            "Assets/file.ext"
        );
        assert_eq!(
            sanitize_path("../../Assets/file.ext").unwrap(),
            "Assets/file.ext"
        );
        assert_eq!(
            sanitize_path("Assets/./file.ext").unwrap(),
            "Assets/file.ext"
        );
        assert_eq!(sanitize_path("Assets/Folder/").unwrap(), "Assets/Folder");
    }

    #[test]
    fn test_parent_component_after_start_is_refused() {
        assert!(sanitize_path("Assets/..").is_err());
        assert!(sanitize_path("Assets\\..\\..\\etc\\passwd").is_err());
    }
}
