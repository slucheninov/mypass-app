pub(crate) mod decode;
pub(crate) mod encode;
pub(crate) mod pool;
mod raw;
mod time;
pub(crate) fn validate_document(xml: &[u8]) -> crate::Result<()> {
    raw::parse(xml, &mut |_| {}).map(|_| ())
}
