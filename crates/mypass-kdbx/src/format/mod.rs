//! Low-level KDBX 4 format pieces: parsing, header authentication and VariantDictionary.

pub(crate) mod header_auth;
pub(crate) mod kdf;
pub(crate) mod limits;
pub(crate) mod outer_header;
pub(crate) mod reader;
pub(crate) mod signature;
pub mod variant_dict;
