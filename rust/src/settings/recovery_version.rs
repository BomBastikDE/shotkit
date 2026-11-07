//! Shared strict source-format check for Windows recovery and load outcomes.
use serde::Deserialize;
use std::io;

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

pub(super) fn validate_v1_source(actual: &[u8]) -> io::Result<()> {
    #[derive(Deserialize)]
    struct VersionHeader {
        version: u32,
    }
    // Reject absent, duplicate, noninteger and malformed headers. A textual
    // occurrence of version 1 is never sufficient authorization.
    if actual
        .iter()
        .copied()
        .find(|byte| !byte.is_ascii_whitespace())
        != Some(b'{')
    {
        return Err(invalid("source must be a JSON object with a version field"));
    }
    let header: VersionHeader = serde_json::from_slice(&actual)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if header.version != 1 {
        return Err(invalid("source is not unambiguously version 1"));
    }
    Ok(())
}
