use std::path::{Path, PathBuf};

const UNIX_PATH_PREFIX: &str = "unix:";

pub(super) fn encode(path: &Path) -> String {
    format!("{UNIX_PATH_PREFIX}{}", hex::encode(path_bytes(path)))
}

pub(super) fn decode(value: &str) -> PathBuf {
    let Some(encoded) = value.strip_prefix(UNIX_PATH_PREFIX) else {
        return PathBuf::from(value);
    };
    let Ok(bytes) = hex::decode(encoded) else {
        return PathBuf::from(value);
    };
    path_from_bytes(bytes)
}

pub(super) fn display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(unix)]
fn path_bytes(path: &Path) -> &[u8] {
    use std::os::unix::ffi::OsStrExt;

    path.as_os_str().as_bytes()
}

#[cfg(not(unix))]
fn path_bytes(path: &Path) -> &[u8] {
    path.to_str().unwrap_or_default().as_bytes()
}

#[cfg(unix)]
fn path_from_bytes(bytes: Vec<u8>) -> PathBuf {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    PathBuf::from(OsString::from_vec(bytes))
}

#[cfg(not(unix))]
fn path_from_bytes(bytes: Vec<u8>) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_path_round_trips() {
        let path = Path::new("/tmp/music/song.flac");

        assert_eq!(decode(&encode(path)), path);
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_path_round_trips() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let path = PathBuf::from(OsString::from_vec(b"/tmp/music/\xff.flac".to_vec()));

        assert_eq!(decode(&encode(&path)), path);
    }
}
