//! Identity of an existing filesystem object, following aliases like canonicalization does.
use std::{io, path::Path};

/// Volume and file identifier. Valid for comparisons while inspecting existing objects;
/// identifiers can be reused after deletion and are not a persistent trust credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileIdentity {
    pub(crate) volume: u64,
    pub(crate) file: [u8; 16],
}

pub fn file_identity(path: &Path) -> io::Result<FileIdentity> {
    #[cfg(windows)]
    {
        crate::windows::file_identity(path)
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = std::fs::metadata(path)?;
        Ok(FileIdentity {
            volume: meta.dev(),
            file: u128::from(meta.ino()).to_le_bytes(),
        })
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = path;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "filesystem identity unavailable",
        ))
    }
}
