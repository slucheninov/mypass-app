//! Low-level KDBX 4 format pieces: parsing, header authentication and VariantDictionary.

#[allow(dead_code)] // remove in Task 9
pub(crate) mod block_stream;
pub(crate) mod header_auth;
pub(crate) mod kdf;
pub(crate) mod limits;
pub(crate) mod outer_header;
pub(crate) mod reader;
pub(crate) mod signature;
pub mod variant_dict;

#[allow(dead_code)] // remove in Task 9
pub(crate) mod cipher;
#[allow(dead_code)] // remove in Task 9
pub(crate) mod compression;
