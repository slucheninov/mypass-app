//! Authenticated KDBX open pipeline.
use crate::{
    format::{
        block_stream, cipher, compression, header_auth,
        inner_header::InnerHeader,
        inner_stream::InnerStream,
        kdf::{derive_keys, CompositeKey},
        outer_header::OuterHeader,
        reader::Reader,
        xml::decode,
    },
    *,
};
use secrecy::SecretString;
use std::fmt;
use zeroize::Zeroizing;
#[derive(Clone, Debug, PartialEq)]
pub struct SaveSettings {
    pub minor_version: u16,
    pub cipher: Cipher,
    pub compression: Compression,
    pub kdf: KdfParams,
}
#[derive(Clone)]
pub struct Database {
    pub root: Group,
    pub deleted_objects: Vec<DeletedObject>,
    pub meta: Meta,
    pub settings: SaveSettings,
    pub dropped_attachment_refs: usize,
    pub dropped_protected_unknown: usize,
}
impl PartialEq for Database {
    fn eq(&self, o: &Self) -> bool {
        self.root == o.root && self.deleted_objects == o.deleted_objects && self.meta == o.meta
    }
}
impl Database {
    pub fn open(data: &[u8], password: &SecretString) -> Result<Self> {
        let (header, offset) = OuterHeader::parse(data)?;
        let raw = data
            .get(..offset)
            .ok_or_else(|| KdbxError::Corrupt("header offset".into()))?;
        let mut reader = Reader::new(
            data.get(offset..)
                .ok_or_else(|| KdbxError::Corrupt("header offset".into()))?,
        );
        let sha = reader.bytes(32)?;
        let mac = reader.bytes(32)?;
        // Key-independent integrity and KDF limits precede any expensive derivation (R10).
        header_auth::verify_sha256(raw, sha)?;
        let kdf = KdfParams::from_variant(&header.kdf_parameters)?;
        let transformed = kdf.transform(&CompositeKey::from_password(password))?;
        let keys = derive_keys(&header.master_seed, &transformed);
        header_auth::verify_hmac(raw, &keys.hmac_key_base, mac)?;
        let blocks = reader.bytes(reader.remaining())?;
        // Authenticate every block before touching the outer cipher (no padding oracle).
        let encrypted = Zeroizing::new(block_stream::read(blocks, &keys.hmac_key_base)?);
        let decrypted = Zeroizing::new(cipher::decrypt(
            header.cipher,
            &keys.master_key,
            &header.encryption_iv,
            &encrypted,
        )?);
        let plain = Zeroizing::new(match header.compression {
            Compression::None => decrypted.to_vec(),
            Compression::Gzip => compression::gunzip(&decrypted)?,
        });
        let (inner, offset) = InnerHeader::parse(&plain)?;
        let mut stream = InnerStream::new(inner.stream, &inner.stream_key)?;
        let decoded = decode::decode(
            plain
                .get(offset..)
                .ok_or_else(|| KdbxError::Corrupt("inner offset".into()))?,
            &inner.binaries,
            &mut |v| stream.apply(v),
        )?;
        if stream.is_failed() {
            return Err(KdbxError::Corrupt("inner stream exhausted".into()));
        }
        Ok(Self {
            root: decoded.root,
            meta: decoded.meta,
            deleted_objects: decoded.deleted_objects,
            settings: SaveSettings {
                minor_version: header.version.minor,
                cipher: header.cipher,
                compression: header.compression,
                kdf,
            },
            dropped_attachment_refs: decoded.dropped_attachment_refs,
            dropped_protected_unknown: decoded.dropped_protected_unknown,
        })
    }
}
impl fmt::Debug for Database {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn counts(g: &Group) -> (usize, usize) {
            g.groups.iter().fold((g.entries.len(), 1), |(e, n), g| {
                let (a, b) = counts(g);
                (e + a, n + b)
            })
        }
        let (e, g) = counts(&self.root);
        // KDF Debug currently contains salts: format the parameter set explicitly without them.
        let kdf = match &self.settings.kdf {
            KdfParams::AesKdf { rounds, .. } => format!("AES-KDF rounds={rounds}"),
            KdfParams::Argon2 {
                variant, memory_bytes, iterations, parallelism, version, ..
            } => format!(
                "{variant:?} memory={memory_bytes} iterations={iterations} parallelism={parallelism} version={version}"
            ),
        };
        f.debug_struct("Database")
            .field("entries", &e)
            .field("groups", &g)
            .field("deleted_objects", &self.deleted_objects.len())
            .field("kdf", &kdf)
            .field("cipher", &self.settings.cipher)
            .field("compression", &self.settings.compression)
            .finish()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::signature::Version;
    fn make_vector(rounds: u64, bad_sha: bool) -> Vec<u8> {
        let kdf = KdfParams::AesKdf {
            seed: vec![4; 32],
            rounds,
        };
        let header = OuterHeader {
            version: Version { major: 4, minor: 1 },
            cipher: Cipher::Aes256Cbc,
            compression: Compression::None,
            master_seed: vec![5; 32],
            encryption_iv: vec![6; 16],
            kdf_parameters: kdf.to_variant(),
            public_custom_data: None,
        };
        let mut raw = header.serialize();
        let mut sha = header_auth::header_sha256(&raw);
        if bad_sha {
            sha[0] ^= 1;
        }
        let mac = if rounds == 1 {
            let transformed = kdf
                .transform(&CompositeKey::from_password(&SecretString::from(
                    "password",
                )))
                .unwrap();
            header_auth::header_hmac(
                &raw,
                &derive_keys(&header.master_seed, &transformed).hmac_key_base,
            )
        } else {
            [0; 32]
        };
        raw.extend(sha);
        raw.extend(mac);
        // Bad MAC and one-byte ciphertext: decrypting this first would report outer-cipher failure.
        raw.extend([0; 32]);
        raw.extend(1_u32.to_le_bytes());
        raw.push(0);
        raw
    }
    #[test]
    fn block_authentication_precedes_decryption_and_wrong_password_stops_at_header() {
        let wire = make_vector(1, false);
        let err = Database::open(&wire, &SecretString::from("password")).unwrap_err();
        assert!(matches!(err,KdbxError::Corrupt(ref s) if s.contains("HMAC")));
        assert_eq!(
            Database::open(&wire, &SecretString::from("nope")),
            Err(KdbxError::WrongPassword)
        );
    }
    #[test]
    fn header_sha_and_kdf_limits_precede_transform() {
        let rounds = 1_000_000_001;
        let result = Database::open(&make_vector(rounds, true), &SecretString::from("x"));
        assert!(matches!(result, Err(KdbxError::Corrupt(ref s)) if s == "header checksum"));
        assert!(matches!(
            Database::open(&make_vector(rounds, false), &SecretString::from("x")),
            Err(KdbxError::InvalidKdfParameters(_))
        ));
    }
    #[test]
    fn trailing_data_after_authenticated_terminator_is_corrupt() {
        let data = include_bytes!("../tests/fixtures/aeskdf-aes256-none.kdbx");
        let mut wire = data.to_vec();
        wire.push(0);
        let result = Database::open(&wire, &SecretString::from("TestMasterPass123"));
        assert!(matches!(result, Err(KdbxError::Corrupt(ref s)) if s == "trailing block data"));
    }
}
