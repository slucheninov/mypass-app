//! Pipe framing uses Rust's raw Win32 handles, without CRT text translation.
// test-only support code
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use mypass_bridge::codec::{encode_frame, read_frame, MAX_FRAME_BYTES};

#[test]
fn binary_length_prefixes_keep_cr_lf_ctrl_z_and_nul() {
    for len in [10usize, 13, 26] {
        let payload = vec![b'x'; len];
        let encoded = encode_frame(&payload).unwrap();
        assert_eq!(&encoded[..4], &(len as u32).to_le_bytes());
        assert_eq!(
            read_frame(&mut encoded.as_slice(), MAX_FRAME_BYTES)
                .unwrap()
                .unwrap(),
            payload
        );
    }
}

#[cfg(windows)]
#[test]
fn host_exe_has_console_subsystem_by_default() {
    let exe = std::fs::read(env!("CARGO_BIN_EXE_mypass-host")).unwrap();
    assert_eq!(&exe[..2], b"MZ");
    let pe = u32::from_le_bytes(exe[0x3c..0x40].try_into().unwrap()) as usize;
    assert_eq!(&exe[pe..pe + 4], b"PE\0\0");
    let optional = pe + 24;
    assert_eq!(
        u16::from_le_bytes(exe[optional..optional + 2].try_into().unwrap()),
        0x20b
    );
    // IMAGE_OPTIONAL_HEADER64.Subsystem; IMAGE_SUBSYSTEM_WINDOWS_CUI == 3.
    assert_eq!(
        u16::from_le_bytes(exe[optional + 68..optional + 70].try_into().unwrap()),
        3
    );
}
