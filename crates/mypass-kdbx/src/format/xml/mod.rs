mod names;
pub(crate) use names::ParentKind;
pub(crate) mod decode;
pub(crate) mod encode;
pub(crate) mod pool;
mod raw;
pub(crate) use raw::validate_extras;
pub(crate) mod time;
pub(crate) fn validate_document(xml: &[u8]) -> crate::Result<()> {
    raw::parse(xml, &mut |_| {})
        .map(|_| ())
        .map_err(|err| match err {
            crate::KdbxError::Corrupt(reason) => crate::KdbxError::InvalidOperation(reason),
            other => other,
        })
}
