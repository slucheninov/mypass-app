//! Password composite keys, bounded KDF parameters and KDBX derived keys.
use super::{
    limits::*,
    variant_dict::{VariantDictionary, VariantValue},
};
use crate::error::{KdbxError, Result};
use aes::cipher::{BlockCipherEncrypt, KeyInit};
use argon2::{Algorithm, Argon2, Params, Version};
use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest, Sha256, Sha512};
use zeroize::{Zeroize, Zeroizing};

const ARGON2D_UUID: [u8; 16] = [
    0xef, 0x63, 0x6d, 0xdf, 0x8c, 0x29, 0x44, 0x4b, 0x91, 0xf7, 0xa9, 0xa4, 0x03, 0xe3, 0x0a, 0x0c,
];
const ARGON2ID_UUID: [u8; 16] = [
    0x9e, 0x29, 0x8b, 0x19, 0x56, 0xdb, 0x47, 0x73, 0xb2, 0x3d, 0xfc, 0x3e, 0xc6, 0xf0, 0xa1, 0xe6,
];
const AES_UUID: [u8; 16] = [
    0xc9, 0xd9, 0xf3, 0x9a, 0x62, 0x8a, 0x44, 0x60, 0xbf, 0x74, 0x0d, 0x08, 0xc1, 0x8a, 0x4f, 0xea,
];
const AES_LEGACY_UUID: [u8; 16] = [
    0x7c, 0x02, 0xbb, 0x82, 0x79, 0xa7, 0x4a, 0xc0, 0x92, 0x7d, 0x11, 0x4a, 0x00, 0x64, 0x82, 0x38,
];

pub struct CompositeKey(Zeroizing<[u8; 32]>);
impl CompositeKey {
    /// Hash the password's exact UTF-8 bytes twice, without Unicode normalization.
    pub fn from_password(password: &SecretString) -> Self {
        let first = Zeroizing::new(<[u8; 32]>::from(Sha256::digest(
            password.expose_secret().as_bytes(),
        )));
        Self(Zeroizing::new(Sha256::digest(first.as_ref()).into()))
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Argon2Variant {
    D,
    Id,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KdfParams {
    Argon2 {
        variant: Argon2Variant,
        salt: Vec<u8>,
        memory_bytes: u64,
        iterations: u64,
        parallelism: u32,
        version: u32,
    },
    AesKdf {
        seed: Vec<u8>,
        rounds: u64,
    },
}
fn invalid(reason: &str) -> KdbxError {
    KdbxError::InvalidKdfParameters(reason.to_owned())
}

fn bytes<'a>(dict: &'a VariantDictionary, name: &str) -> Result<&'a [u8]> {
    match dict.get(name) {
        Some(VariantValue::Bytes(value)) => Ok(value),
        _ => Err(invalid("missing or wrong-typed byte parameter")),
    }
}
fn u64_param(dict: &VariantDictionary, name: &str) -> Result<u64> {
    match dict.get(name) {
        Some(VariantValue::U64(value)) => Ok(*value),
        _ => Err(invalid("missing or wrong-typed u64 parameter")),
    }
}
fn u32_param(dict: &VariantDictionary, name: &str) -> Result<u32> {
    match dict.get(name) {
        Some(VariantValue::U32(value)) => Ok(*value),
        _ => Err(invalid("missing or wrong-typed u32 parameter")),
    }
}

// These constructors are infallible by contract. Never continue with a zero or partially filled
// salt if the OS RNG fails; panicking here is independent of parsing any untrusted vault input.
fn randomize(bytes: &mut [u8]) {
    if getrandom::fill(bytes).is_err() {
        panic!("OS randomness unavailable");
    }
}

impl KdfParams {
    /// Fresh vault defaults; a new OS-random 32-byte salt is generated on every call.
    pub fn argon2id_default() -> Self {
        let mut salt = vec![0; 32];
        randomize(&mut salt);
        Self::Argon2 {
            variant: Argon2Variant::Id,
            salt,
            memory_bytes: 64 * 1024 * 1024,
            iterations: 3,
            parallelism: 2,
            version: 0x13,
        }
    }

    /// Validate all attacker-controlled costs before KDF work or workspace allocation.
    pub fn from_variant(dict: &VariantDictionary) -> Result<Self> {
        let uuid = bytes(dict, "$UUID")?;
        let params = if uuid == ARGON2D_UUID || uuid == ARGON2ID_UUID {
            // Ignoring either field would silently derive a different key from the file's key.
            if dict.get("K").is_some() || dict.get("A").is_some() {
                return Err(invalid("secret and associated data are unsupported"));
            }
            Self::Argon2 {
                variant: if uuid == ARGON2D_UUID {
                    Argon2Variant::D
                } else {
                    Argon2Variant::Id
                },
                salt: bytes(dict, "S")?.to_vec(),
                memory_bytes: u64_param(dict, "M")?,
                iterations: u64_param(dict, "I")?,
                parallelism: u32_param(dict, "P")?,
                version: u32_param(dict, "V")?,
            }
        } else if uuid == AES_UUID || uuid == AES_LEGACY_UUID {
            Self::AesKdf {
                seed: bytes(dict, "S")?.to_vec(),
                rounds: u64_param(dict, "R")?,
            }
        } else {
            return Err(invalid("unsupported KDF UUID"));
        };
        params.validate()?;
        Ok(params)
    }

    fn validate(&self) -> Result<()> {
        match self {
            Self::Argon2 {
                salt,
                memory_bytes,
                iterations,
                parallelism,
                version,
                ..
            } => {
                if !(ARGON2_MEMORY_MIN_BYTES..=ARGON2_MEMORY_MAX_BYTES).contains(memory_bytes)
                    || memory_bytes % 1024 != 0
                    || !(ARGON2_ITERATIONS_MIN..=ARGON2_ITERATIONS_MAX).contains(iterations)
                    || !(ARGON2_PARALLELISM_MIN..=ARGON2_PARALLELISM_MAX).contains(parallelism)
                    || salt.len() < ARGON2_SALT_MIN_LEN
                    || memory_bytes / 1024 < 8 * u64::from(*parallelism)
                {
                    return Err(invalid("Argon2 cost or salt outside limits"));
                }
                Version::try_from(*version).map_err(|_| invalid("unsupported Argon2 version"))?;
            }
            Self::AesKdf { seed, rounds } => {
                if seed.len() != 32 || !(AES_KDF_ROUNDS_MIN..=AES_KDF_ROUNDS_MAX).contains(rounds) {
                    return Err(invalid("AES-KDF seed length or rounds outside limits"));
                }
            }
        }
        Ok(())
    }

    pub fn to_variant(&self) -> VariantDictionary {
        let mut dict = VariantDictionary::default();
        match self {
            Self::Argon2 {
                variant,
                salt,
                memory_bytes,
                iterations,
                parallelism,
                version,
            } => {
                let uuid = match variant {
                    Argon2Variant::D => ARGON2D_UUID,
                    Argon2Variant::Id => ARGON2ID_UUID,
                };
                dict.set("$UUID", VariantValue::Bytes(uuid.to_vec()));
                dict.set("S", VariantValue::Bytes(salt.clone()));
                dict.set("M", VariantValue::U64(*memory_bytes));
                dict.set("I", VariantValue::U64(*iterations));
                dict.set("P", VariantValue::U32(*parallelism));
                dict.set("V", VariantValue::U32(*version));
            }
            Self::AesKdf { seed, rounds } => {
                dict.set("$UUID", VariantValue::Bytes(AES_UUID.to_vec()));
                dict.set("S", VariantValue::Bytes(seed.clone()));
                dict.set("R", VariantValue::U64(*rounds));
            }
        }
        dict
    }

    /// Preserve the KDF and costs on save, replacing only the salt/seed with OS randomness.
    pub fn with_fresh_randomness(&self) -> Self {
        let mut fresh = self.clone();
        match &mut fresh {
            Self::Argon2 { salt, .. } => randomize(salt),
            Self::AesKdf { seed, .. } => randomize(seed),
        }
        fresh
    }

    pub fn transform(&self, key: &CompositeKey) -> Result<Zeroizing<[u8; 32]>> {
        self.transform_bytes(key.0.as_ref())
    }

    // Kept as a real internal function so the independent Swift raw-input KATs exercise the
    // identical dispatch and implementation as transform, without changing their 28-byte input.
    fn transform_bytes(&self, key: &[u8]) -> Result<Zeroizing<[u8; 32]>> {
        // Public enum variants can be constructed directly: revalidate before expensive work.
        self.validate()?;
        match self {
            Self::Argon2 {
                variant,
                salt,
                memory_bytes,
                iterations,
                parallelism,
                version,
            } => {
                let algorithm = match variant {
                    Argon2Variant::D => Algorithm::Argon2d,
                    Argon2Variant::Id => Algorithm::Argon2id,
                };
                let memory_kib = u32::try_from(memory_bytes / 1024)
                    .map_err(|_| invalid("Argon2 memory conversion"))?;
                let iterations = u32::try_from(*iterations)
                    .map_err(|_| invalid("Argon2 iteration conversion"))?;
                let params = Params::new(memory_kib, iterations, *parallelism, Some(32))
                    .map_err(|_| invalid("invalid Argon2 costs"))?;
                let version = Version::try_from(*version)
                    .map_err(|_| invalid("unsupported Argon2 version"))?;
                let mut output = Zeroizing::new([0; 32]);
                // The crate's allocating convenience API does not wipe its workspace on drop.
                // Own that storage explicitly so intermediate password-derived blocks are erased
                // on both success and error, and report allocation failure instead of panicking.
                let mut memory = Zeroizing::new(Vec::new());
                memory
                    .try_reserve_exact(params.block_count())
                    .map_err(|_| invalid("Argon2 workspace allocation failed"))?;
                memory.resize(params.block_count(), argon2::Block::default());
                Argon2::new(algorithm, version, params)
                    .hash_password_into_with_memory(
                        key,
                        salt,
                        output.as_mut(),
                        memory.as_mut_slice(),
                    )
                    .map_err(|_| invalid("Argon2 derivation failed"))?;
                Ok(output)
            }
            Self::AesKdf { seed, rounds } => {
                let cipher = aes::Aes256::new_from_slice(seed)
                    .map_err(|_| invalid("AES-KDF seed length"))?;
                let mut data = Zeroizing::new(
                    <[u8; 32]>::try_from(key).map_err(|_| invalid("AES-KDF input length"))?,
                );
                // Each round encrypts both independent 16-byte blocks (ECB), with no padding.
                let (left, right) = data.split_at_mut(16);
                let mut left_block = aes::Block::default();
                let mut right_block = aes::Block::default();
                left_block.copy_from_slice(left);
                right_block.copy_from_slice(right);
                for _ in 0..*rounds {
                    cipher.encrypt_block(&mut left_block);
                    cipher.encrypt_block(&mut right_block);
                }
                left.copy_from_slice(&left_block);
                right.copy_from_slice(&right_block);
                left_block.as_mut_slice().zeroize();
                right_block.as_mut_slice().zeroize();
                Ok(Zeroizing::new(Sha256::digest(data.as_ref()).into()))
            }
        }
    }
}

pub struct DerivedKeys {
    pub master_key: Zeroizing<[u8; 32]>,
    pub hmac_key_base: Zeroizing<[u8; 64]>,
}

pub fn derive_keys(master_seed: &[u8], transformed: &[u8; 32]) -> DerivedKeys {
    let mut master = Sha256::new();
    master.update(master_seed);
    master.update(transformed);
    let mut hmac = Sha512::new();
    hmac.update(master_seed);
    hmac.update(transformed);
    hmac.update([1]);
    DerivedKeys {
        master_key: Zeroizing::new(master.finalize().into()),
        hmac_key_base: Zeroizing::new(hmac.finalize().into()),
    }
}

#[cfg(test)]
mod tests {
    // Test-only support code: assertions may unwrap and index fixed fixtures.
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
    use super::*;

    fn argon(variant: Argon2Variant, parallelism: u32, salt_len: u8) -> KdfParams {
        KdfParams::Argon2 {
            variant,
            salt: (0..salt_len).collect(),
            memory_bytes: 19 * 1024 * 1024,
            iterations: 2,
            parallelism,
            version: 0x13,
        }
    }
    fn aes() -> KdfParams {
        KdfParams::AesKdf {
            seed: (0..32).collect(),
            rounds: 3,
        }
    }
    fn assert_invalid(params: &KdfParams) {
        assert!(matches!(
            KdfParams::from_variant(&params.to_variant()),
            Err(KdbxError::InvalidKdfParameters(_))
        ));
        assert!(matches!(
            params.transform(&CompositeKey(Zeroizing::new([0; 32]))),
            Err(KdbxError::InvalidKdfParameters(_))
        ));
    }
    fn check_argon(variant: Argon2Variant, lanes: u32, salt_len: u8, expected: [u8; 32]) {
        let dict = argon(variant, lanes, salt_len).to_variant();
        // Pin the wire UUID independently: a matching serializer/parser swap must not pass.
        let uuid = match variant {
            Argon2Variant::D => vec![
                0xef, 0x63, 0x6d, 0xdf, 0x8c, 0x29, 0x44, 0x4b, 0x91, 0xf7, 0xa9, 0xa4, 0x03, 0xe3,
                0x0a, 0x0c,
            ],
            Argon2Variant::Id => vec![
                0x9e, 0x29, 0x8b, 0x19, 0x56, 0xdb, 0x47, 0x73, 0xb2, 0x3d, 0xfc, 0x3e, 0xc6, 0xf0,
                0xa1, 0xe6,
            ],
        };
        assert_eq!(dict.get("$UUID"), Some(&VariantValue::Bytes(uuid)));
        let params = KdfParams::from_variant(&dict).unwrap();
        // Swift vectors deliberately use raw 28-byte input, not a password composite hash.
        assert_eq!(
            *params
                .transform_bytes(b"correct horse battery staple")
                .unwrap(),
            expected
        );
    }
    #[test]
    fn argon2id_known_answer() {
        check_argon(
            Argon2Variant::Id,
            1,
            16,
            [
                0x81, 0x82, 0x59, 0xb6, 0x31, 0x00, 0x26, 0xa8, 0xe0, 0xdb, 0xac, 0x5d, 0x2e, 0x69,
                0x27, 0xab, 0xcf, 0xdb, 0x07, 0xb3, 0x22, 0x58, 0xfa, 0xc4, 0xf6, 0x1b, 0x18, 0xb8,
                0x0f, 0x92, 0x90, 0x85,
            ],
        );
    }
    #[test]
    fn argon2d_known_answer() {
        check_argon(
            Argon2Variant::D,
            1,
            16,
            [
                0x4c, 0x26, 0x81, 0xb4, 0x2c, 0x5b, 0x81, 0x96, 0x22, 0xa8, 0x88, 0x71, 0x5e, 0xaf,
                0x8f, 0xb1, 0x45, 0x73, 0x21, 0x10, 0x18, 0x3c, 0x09, 0x51, 0x72, 0x4b, 0xa2, 0x59,
                0x5a, 0x9a, 0x1f, 0xde,
            ],
        );
    }
    #[test]
    fn argon2_parallelism_two_and_four_known_answers() {
        check_argon(
            Argon2Variant::Id,
            2,
            16,
            [
                0xd8, 0x7f, 0x42, 0x24, 0x13, 0x02, 0xfe, 0x7d, 0x74, 0xbf, 0xac, 0xd8, 0x66, 0xc9,
                0x24, 0x57, 0xa4, 0x96, 0x99, 0xe8, 0xa5, 0x0d, 0x19, 0xac, 0x8b, 0x46, 0x27, 0x3b,
                0x2c, 0x58, 0x1a, 0x90,
            ],
        );
        check_argon(
            Argon2Variant::Id,
            4,
            16,
            [
                0xb9, 0x80, 0x9d, 0x33, 0x24, 0x60, 0x01, 0x14, 0x00, 0x27, 0x2c, 0xf6, 0x70, 0x66,
                0xfc, 0x28, 0x00, 0xf1, 0x70, 0xcf, 0x06, 0x6c, 0x02, 0xe2, 0x20, 0x20, 0xc1, 0xf3,
                0xd1, 0x6f, 0xb5, 0x76,
            ],
        );
    }
    #[test]
    fn argon2_32_byte_salt_known_answer() {
        check_argon(
            Argon2Variant::Id,
            1,
            32,
            [
                0x09, 0x2d, 0x6e, 0x91, 0x98, 0x78, 0x40, 0xe6, 0x3e, 0x2f, 0xac, 0x5e, 0x18, 0x7a,
                0xc5, 0xd2, 0x9b, 0x48, 0x9f, 0x05, 0x59, 0x79, 0x71, 0xfd, 0x65, 0x54, 0x55, 0x5a,
                0x1a, 0x20, 0xce, 0x2a,
            ],
        );
    }
    #[test]
    fn aes_kdf_known_answer() {
        let mut bytes = [0xaa; 32];
        bytes[16..].fill(0xbb);
        assert_eq!(
            *aes()
                .transform(&CompositeKey(Zeroizing::new(bytes)))
                .unwrap(),
            [
                0xdf, 0x34, 0xca, 0x11, 0x6b, 0x7e, 0x1c, 0x04, 0xcb, 0x4b, 0x60, 0xbc, 0x81, 0x3c,
                0x60, 0xab, 0xfd, 0x27, 0xba, 0x93, 0x34, 0xbd, 0xd6, 0xa4, 0x89, 0xac, 0xb4, 0x9a,
                0xe2, 0x4d, 0xa9, 0xb4,
            ]
        );
    }
    #[test]
    fn aes_kdf_accepts_legacy_kdbx3_uuid() {
        // The brief's "legacy" naming is inverted: KeePassXC's KDF_AES_KDBX3 (c9d9f39a) is emitted; 7c02bb82 is also accepted.
        let emitted_uuid = vec![
            0xc9, 0xd9, 0xf3, 0x9a, 0x62, 0x8a, 0x44, 0x60, 0xbf, 0x74, 0x0d, 0x08, 0xc1, 0x8a,
            0x4f, 0xea,
        ];
        let other_uuid = vec![
            0x7c, 0x02, 0xbb, 0x82, 0x79, 0xa7, 0x4a, 0xc0, 0x92, 0x7d, 0x11, 0x4a, 0x00, 0x64,
            0x82, 0x38,
        ];
        assert_eq!(
            aes().to_variant().get("$UUID"),
            Some(&VariantValue::Bytes(emitted_uuid.clone()))
        );
        for uuid in [emitted_uuid, other_uuid] {
            let mut dict = VariantDictionary::default();
            dict.set("$UUID", VariantValue::Bytes(uuid));
            dict.set("S", VariantValue::Bytes((0..32).collect()));
            dict.set("R", VariantValue::U64(3));
            let params = KdfParams::from_variant(&dict).unwrap();
            assert_eq!(params, aes());
            let key = CompositeKey(Zeroizing::new([5; 32]));
            assert_eq!(
                *params.transform(&key).unwrap(),
                *aes().transform(&key).unwrap()
            );
        }
    }
    #[test]
    fn aes_kdf_missing_params_is_invalid() {
        for missing in ["S", "R"] {
            let mut dict = aes().to_variant();
            dict.items.retain(|(k, _)| k != missing);
            assert!(matches!(
                KdfParams::from_variant(&dict),
                Err(KdbxError::InvalidKdfParameters(_))
            ));
        }
    }
    #[test]
    fn aes_kdf_wrong_seed_length_is_invalid() {
        assert_invalid(&KdfParams::AesKdf {
            seed: vec![0; 16],
            rounds: 3,
        });
    }
    #[test]
    fn unsupported_kdf_uuid_is_invalid() {
        let mut dict = aes().to_variant();
        dict.set("$UUID", VariantValue::Bytes(vec![255; 16]));
        assert!(matches!(
            KdfParams::from_variant(&dict),
            Err(KdbxError::InvalidKdfParameters(_))
        ));
    }
    fn changed(key: &str, value: VariantValue) {
        let mut dict = argon(Argon2Variant::Id, 1, 16).to_variant();
        dict.set(key, value);
        assert!(matches!(
            KdfParams::from_variant(&dict),
            Err(KdbxError::InvalidKdfParameters(_))
        ));
    }
    #[test]
    fn parallelism_zero_and_256_rejected() {
        for p in [0, 256] {
            changed("P", VariantValue::U32(p));
        }
    }
    #[test]
    fn memory_below_8_kib_and_above_4_gib_rejected() {
        // Non-multiples also fail the % 1024 rule; multiples isolate the range rule (P = 1).
        for m in [
            8191,
            4 * 1024 * 1024 * 1024 + 1,
            u64::MAX,
            8193,
            7 * 1024,
            4 * 1024 * 1024 * 1024 + 1024,
            1 << 40,
        ] {
            changed("M", VariantValue::U64(m));
        }
    }
    #[test]
    fn memory_below_eight_times_parallelism_rejected() {
        assert_invalid(&KdfParams::Argon2 {
            variant: Argon2Variant::Id,
            salt: vec![0; 16],
            memory_bytes: 8 * 1024,
            iterations: 1,
            parallelism: 2,
            version: 0x13,
        });
    }
    #[test]
    fn iterations_zero_and_10_001_rejected() {
        for i in [0, 10_001, u64::MAX] {
            changed("I", VariantValue::U64(i));
        }
    }
    #[test]
    fn salt_shorter_than_8_rejected() {
        changed("S", VariantValue::Bytes(vec![0; 7]));
    }
    #[test]
    fn secret_and_associated_data_rejected() {
        for k in ["K", "A"] {
            changed(k, VariantValue::Bytes(vec![]));
        }
    }
    #[test]
    fn aes_kdf_rounds_above_cap_rejected() {
        for rounds in [0, 1_000_000_001, u64::MAX] {
            assert_invalid(&KdfParams::AesKdf {
                seed: vec![0; 32],
                rounds,
            });
        }
    }
    #[test]
    fn missing_and_wrong_typed_argon_fields_rejected() {
        for k in ["$UUID", "S", "M", "I", "P", "V"] {
            let mut d = argon(Argon2Variant::Id, 1, 16).to_variant();
            d.items.retain(|(name, _)| name != k);
            assert!(matches!(
                KdfParams::from_variant(&d),
                Err(KdbxError::InvalidKdfParameters(_))
            ));
            changed(k, VariantValue::Bool(false));
        }
        changed("V", VariantValue::U32(0x12));
    }
    #[test]
    fn valid_limit_edges_and_versions_parse_without_running_expensive_kdf() {
        for (key, value) in [
            ("M", VariantValue::U64(8192)),
            ("M", VariantValue::U64(4 * 1024 * 1024 * 1024)),
            ("I", VariantValue::U64(10_000)),
            ("P", VariantValue::U32(255)),
            ("S", VariantValue::Bytes(vec![0; 8])),
            ("V", VariantValue::U32(0x10)),
        ] {
            let mut d = argon(Argon2Variant::Id, 1, 16).to_variant();
            d.set(key, value);
            assert!(KdfParams::from_variant(&d).is_ok());
        }
        for rounds in [1, 1_000_000_000] {
            assert!(KdfParams::from_variant(
                &KdfParams::AesKdf {
                    seed: vec![0; 32],
                    rounds
                }
                .to_variant()
            )
            .is_ok());
        }
    }
    #[test]
    fn master_and_hmac_key_base_differ() {
        let keys = derive_keys(&[1; 32], &[2; 32]);
        assert_ne!(&keys.master_key[..], &keys.hmac_key_base[..32]);
        assert_eq!(
            *keys.master_key,
            <[u8; 32]>::from(Sha256::digest([[1; 32], [2; 32]].concat()))
        );
        let mut material = vec![1; 32];
        material.extend([2; 32]);
        material.push(1);
        assert_eq!(
            *keys.hmac_key_base,
            <[u8; 64]>::from(Sha512::digest(material))
        );
    }
    #[test]
    fn with_fresh_randomness_keeps_params_and_salt_length_but_changes_salt() {
        for params in [
            argon(Argon2Variant::D, 4, 16),
            argon(Argon2Variant::Id, 2, 32),
            aes(),
        ] {
            let fresh = params.with_fresh_randomness();
            let mut old = params.to_variant();
            let mut new = fresh.to_variant();
            let old_salt = old.items.iter().find(|(k, _)| k == "S").unwrap().1.clone();
            let new_salt = new.items.iter().find(|(k, _)| k == "S").unwrap().1.clone();
            assert_ne!(old_salt, new_salt);
            if let (VariantValue::Bytes(a), VariantValue::Bytes(b)) = (old_salt, new_salt) {
                assert_eq!(a.len(), b.len());
            } else {
                panic!("salt type");
            }
            old.items.retain(|(k, _)| k != "S");
            new.items.retain(|(k, _)| k != "S");
            assert_eq!(old, new);
            assert_eq!(KdfParams::from_variant(&fresh.to_variant()).unwrap(), fresh);
        }
        let default = KdfParams::argon2id_default();
        assert!(
            matches!(default, KdfParams::Argon2 {variant:Argon2Variant::Id,ref salt,memory_bytes:67_108_864,iterations:3,parallelism:2,version:0x13} if salt.len()==32)
        );
        assert_ne!(default, KdfParams::argon2id_default());
    }
    #[test]
    fn composite_key_utf8_bytes() {
        let password = "\u{43f}\u{440}\u{438}\u{432}\u{456}\u{442}";
        assert_eq!(
            *CompositeKey::from_password(&SecretString::from(password)).0,
            <[u8; 32]>::from(Sha256::digest(Sha256::digest(password.as_bytes())))
        );
    }
    #[test]
    fn nfc_and_nfd_passwords_are_different_keys() {
        assert_ne!(
            *CompositeKey::from_password(&SecretString::from("\u{439}")).0,
            *CompositeKey::from_password(&SecretString::from("\u{438}\u{306}")).0
        );
    }
}
