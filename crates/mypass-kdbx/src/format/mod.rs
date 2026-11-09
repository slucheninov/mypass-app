//! Low-level KDBX 4 format pieces: parsing, header authentication and VariantDictionary.

pub(crate) mod block_stream;
pub(crate) mod header_auth;
pub(crate) mod kdf;
pub(crate) mod limits;
pub(crate) mod outer_header;
pub(crate) mod reader;
pub(crate) mod signature;
pub(crate) mod variant_dict;

pub(crate) mod cipher;
pub(crate) mod compression;

pub(crate) mod inner_header;
pub(crate) mod inner_stream;

pub(crate) mod xml;
