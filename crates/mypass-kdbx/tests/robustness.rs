// Test-only support code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
use mypass_kdbx::*;
use rand::{rngs::StdRng, Rng, SeedableRng};
use secrecy::SecretString;
const DATA: &[u8] = include_bytes!("fixtures/aeskdf-aes256-none.kdbx");
fn assert_open(data: &[u8]) -> Result<Database> {
    Database::open(data, &SecretString::from("TestMasterPass123"))
}
fn header_end() -> usize {
    let mut p = 12;
    loop {
        let id = DATA[p];
        let n = u32::from_le_bytes(DATA[p + 1..p + 5].try_into().unwrap()) as usize;
        p += 5 + n;
        if id == 0 {
            return p;
        }
    }
}
#[test]
fn truncated_at_every_prefix_never_panics() {
    for n in (0..=200).chain((201..DATA.len()).step_by(61)) {
        assert!(assert_open(&DATA[..n]).is_err(), "prefix {n}");
    }
}
#[test]
fn single_bit_flips_never_panic() {
    for i in 0..64 {
        let pos = i * (DATA.len() - 1) / 63;
        let mut data = DATA.to_vec();
        data[pos] ^= 1;
        let r = assert_open(&data);
        if pos < header_end() {
            assert!(r.is_err());
        }
    }
}
#[test]
fn random_garbage_never_panics_or_hangs() {
    let mut rng = StdRng::seed_from_u64(9);
    for i in 0..200 {
        let n = rng.gen_range(0..4096);
        let mut data = vec![0; n];
        rng.fill(data.as_mut_slice());
        if i % 3 == 0 && n >= 12 {
            data[..12].copy_from_slice(&DATA[..12]);
        }
        assert!(assert_open(&data).is_err());
    }
}
#[test]
fn kdbx3_file_is_unsupported_version() {
    let mut data = DATA[..12].to_vec();
    data[10..12].copy_from_slice(&3u16.to_le_bytes());
    assert!(matches!(
        assert_open(&data),
        Err(KdbxError::UnsupportedVersion { major: 3, .. })
    ));
}
#[test]
fn wrong_password_vs_corrupt_body() {
    assert_eq!(
        Database::open(DATA, &SecretString::from("wrong")),
        Err(KdbxError::WrongPassword)
    );
    let mut data = DATA.to_vec();
    data[header_end() + 64 + 36] ^= 1;
    assert!(matches!(assert_open(&data), Err(KdbxError::Corrupt(_))));
}
