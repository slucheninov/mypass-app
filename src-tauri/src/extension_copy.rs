//! Keep Chrome's writable unpacked extension outside the signed application bundle.
use std::{
    fs, io,
    path::{Path, PathBuf},
};

/// Copy once per version. The signed source is only ever read; Chrome may edit the copy.
pub(crate) fn writable_extension(source: &Path, data: &Path, version: &str) -> io::Result<PathBuf> {
    let destination = data.join(format!("extension-{version}"));
    if destination.join("manifest.json").is_file() {
        return Ok(destination);
    }
    fs::create_dir_all(data)?;
    let temporary = data.join(format!(".extension-{}", uuid::Uuid::new_v4()));
    let result = (|| {
        copy_tree(source, &temporary)?;
        if !temporary.join("manifest.json").is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "extension manifest missing",
            ));
        }
        match fs::rename(&temporary, &destination) {
            Ok(()) => Ok(destination.clone()),
            Err(_) if destination.join("manifest.json").is_file() => Ok(destination.clone()),
            Err(error) => Err(error),
        }
    })();
    // Only our private, randomly named staging directory is removed, never user data.
    let _ = fs::remove_dir_all(&temporary);
    result
}

fn copy_tree(source: &Path, destination: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "extension symlink refused",
        ));
    }
    if metadata.is_dir() {
        fs::create_dir(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_tree(&entry.path(), &destination.join(entry.file_name()))?;
        }
    } else if metadata.is_file() {
        fs::copy(source, destination)?;
    } else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "extension special file refused",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_copy_is_per_version_and_leaves_bundle_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("bundle/extension");
        fs::create_dir_all(source.join("shared")).unwrap();
        fs::write(source.join("manifest.json"), "manifest").unwrap();
        fs::write(source.join("shared/code.js"), "code").unwrap();
        let data = dir.path().join("data");
        let copy = writable_extension(&source, &data, "0.7.0").unwrap();
        assert_eq!(copy, data.join("extension-0.7.0"));
        fs::write(copy.join("shared/code.js"), "Chrome modified this").unwrap();
        assert_eq!(
            fs::read_to_string(source.join("shared/code.js")).unwrap(),
            "code"
        );
        assert_eq!(writable_extension(&source, &data, "0.7.0").unwrap(), copy);
        assert_eq!(
            fs::read_to_string(copy.join("shared/code.js")).unwrap(),
            "Chrome modified this"
        );
        let next = writable_extension(&source, &data, "0.7.1").unwrap();
        assert_eq!(
            fs::read_to_string(next.join("shared/code.js")).unwrap(),
            "code"
        );
    }

    #[cfg(unix)]
    #[test]
    fn extension_copy_refuses_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("extension");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("manifest.json"), "manifest").unwrap();
        std::os::unix::fs::symlink("/etc/passwd", source.join("outside")).unwrap();
        assert!(writable_extension(&source, &dir.path().join("data"), "0.7.0").is_err());
    }
}
