//! Windows lexical policy is portable; filesystem identity follows the current filesystem.
use mypass_platform::{
    file_identity,
    windows_paths::{validate_component, validate_path},
};

#[test]
fn windows_local_paths_refuse_namespace_aliases_and_devices() {
    for name in [
        "CON",
        "con.kdbx",
        "NUL",
        "AUX",
        "PRN.txt",
        "COM1",
        "LPT9.txt",
        "COM¹",
        "file.kdbx.",
        "file ",
        "x:y",
        "a?b",
        "a\0b",
    ] {
        assert!(validate_component(name).is_err(), "{name:?}");
    }
    for path in [
        r"\\.\C:\vaults",
        r"\\?\GLOBALROOT\Device\HarddiskVolume1",
        r"C:\vaults\NUL",
        r"C:\vaults\file.",
        r"C:\vaults\x:stream",
        r"C:relative",
        r"\rooted",
        r"\\host",
        r"\\?\C:\vaults\..\copy",
    ] {
        assert!(validate_path(path).is_err(), "{path:?}");
    }
    for path in [
        r"C:\",
        r"c:\Vaults\.",
        r"C:/Vaults/../Copy/",
        "\\\\host\\share\\\u{041f}\u{0430}\u{0440}\u{043e}\u{043b}\u{0456} \u{1f511}",
        r"\\?\C:\Vaults",
        r"\\?\UNC\host\share\Vaults\",
    ] {
        assert!(validate_path(path).is_ok(), "{path:?}");
    }
    for name in [
        "Console.kdbx",
        "COM10",
        "\u{041f}\u{0430}\u{0440}\u{043e}\u{043b}\u{0456} \u{1f511}.kdbx",
    ] {
        assert!(validate_component(name).is_ok());
    }
}

#[test]
#[cfg_attr(
    windows,
    ignore = "native Windows identity; isolated integration runner only"
)]
fn windows_integration_filesystem_identity_follows_links_and_distinguishes_copies() {
    #[cfg(windows)]
    assert_eq!(
        std::env::var("MYPASS_WINDOWS_INTEGRATION_TESTS").as_deref(),
        Ok("1")
    );
    let d = tempfile::tempdir().unwrap();
    let a = d.path().join("a");
    let b = d.path().join("b");
    let c = d.path().join("c");
    std::fs::write(&a, b"same").unwrap();
    std::fs::hard_link(&a, &b).unwrap();
    std::fs::copy(&a, &c).unwrap();
    assert_eq!(file_identity(&a).unwrap(), file_identity(&b).unwrap());
    assert_ne!(file_identity(&a).unwrap(), file_identity(&c).unwrap());
    assert!(file_identity(&d.path().join("missing")).is_err());
    assert_eq!(
        file_identity(d.path()).unwrap(),
        file_identity(&d.path().join(".")).unwrap()
    );
}
