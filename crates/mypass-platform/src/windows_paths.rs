//! Lexical Windows local-file boundary policy, testable on every OS. This is validation,
//! never filesystem identity: aliases must still be checked with `file_identity`.
use std::io;

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "invalid Windows local-file path",
    )
}

/// Reject Win32 device basenames (also with extensions), ADS, wildcards and trimmed aliases.
pub fn validate_component(name: &str) -> io::Result<()> {
    if name.is_empty()
        || name.ends_with(['.', ' '])
        || name
            .chars()
            .any(|c| c < ' ' || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'))
    {
        return Err(invalid());
    }
    let base = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    let numbered = ["COM", "LPT"].iter().any(|prefix| {
        base.strip_prefix(prefix).is_some_and(|n| {
            matches!(
                n,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    });
    if numbered
        || matches!(
            base.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        )
    {
        return Err(invalid());
    }
    Ok(())
}

/// Accept only absolute drive/UNC paths, including their verbatim forms. Normal paths may
/// contain dot components; verbatim ones cannot (the OS would treat them literally).
pub fn validate_path(raw: &str) -> io::Result<()> {
    let raw = raw.replace('/', "\\");
    let (verbatim, rest) = match raw.strip_prefix("\\\\?\\") {
        Some(rest) => (true, rest),
        None => (false, raw.as_str()),
    };
    let unc = if verbatim {
        rest.strip_prefix("UNC\\")
    } else {
        rest.strip_prefix("\\\\")
    };
    let tail = if let Some(unc) = unc {
        let mut parts = unc.split('\\');
        validate_component(parts.next().ok_or_else(invalid)?)?;
        validate_component(parts.next().ok_or_else(invalid)?)?;
        parts.collect::<Vec<_>>().join("\\")
    } else {
        let mut chars = rest.chars();
        if !matches!((chars.next(), chars.next(), chars.next()), (Some(c), Some(':'), Some('\\')) if c.is_ascii_alphabetic())
        {
            return Err(invalid());
        }
        chars.collect()
    };
    for part in tail.split('\\').filter(|p| !p.is_empty()) {
        if !verbatim && matches!(part, "." | "..") {
            continue;
        }
        validate_component(part)?;
    }
    Ok(())
}
