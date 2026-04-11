/*!
Search text generating utilities.
*/
use std::{
    borrow::Cow,
    ffi::{OsStr, OsString},
    fs, io,
    path::{Path, PathBuf},
};

/// Canonicalize a path into the form can be recognized by Everything.
#[doc(alias = "realpath_ev")]
pub fn canonicalize_path_ev<'a>(path: &'a Path) -> io::Result<PathBuf> {
    let path = fs::canonicalize(path)?;
    let path = normalize_path_ev(&path).into_owned();
    Ok(path)
}

/// Convert a path into the form can be recognized by Everything.
///
/// Basically removes the verbatim prefix, e.g. `\\?\C:\Windows` -> `C:\Windows`.
/// Note that this does not resolve symbolic links and junctions,
/// for which you can call [`canonicalize_path_ev()`].
///
/// https://github.com/dylni/normpath can also be used, but a bit overkill.
pub fn normalize_path_ev<'a>(path: &'a Path) -> Cow<'a, Path> {
    let bytes = path.as_os_str().as_encoded_bytes();
    if let Some(bytes) = bytes.strip_prefix(br"\\?\") {
        // `\\?\UNC\server\share` -> `\\server\share`
        if let Some(bytes) = bytes.strip_prefix(b"UNC") {
            let s = unsafe { OsStr::from_encoded_bytes_unchecked(bytes) };
            let mut buf = OsString::with_capacity(1 + s.len());
            buf.push(OsStr::new("\\"));
            buf.push(s);
            return Cow::Owned(PathBuf::from(buf));
        }

        // `\\?\C:` -> `C:`
        // Ignore `\\?\cat_pics`
        let s = unsafe { OsStr::from_encoded_bytes_unchecked(bytes) };
        return Cow::Borrowed(Path::new(s));
    }
    Cow::Borrowed(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_path_ev_verbatim_disk() {
        // Verbatim disk path: \\?\C:\foo -> C:\foo
        let path = Path::new(r"\\?\C:\Windows");
        let result = normalize_path_ev(path);
        assert_eq!(result, Path::new(r"C:\Windows"));
    }

    #[test]
    fn normalize_path_ev_regular_disk() {
        // Regular disk path returns as-is (not verbatim)
        let path = Path::new(r"C:\Windows");
        let result = normalize_path_ev(path);
        assert_eq!(result, Path::new(r"C:\Windows"));
    }

    #[test]
    fn normalize_path_ev_verbatim_unc() {
        // Verbatim UNC path: \\?\UNC\server\share -> \\server\share
        let path = Path::new(r"\\?\UNC\server\share");
        let result = normalize_path_ev(path);
        assert_eq!(result, Path::new(r"\\server\share"));
    }

    #[test]
    fn normalize_path_ev_relative() {
        // Relative paths return as-is
        let path = Path::new("foo/bar");
        let result = normalize_path_ev(path);
        assert_eq!(result, Path::new("foo/bar"));
    }

    #[test]
    fn normalize_path_ev_root_only() {
        // Verbatim disk path to root
        let path = Path::new(r"\\?\C:\");
        let result = normalize_path_ev(path);
        assert_eq!(result, Path::new(r"C:\"));
    }
}
