//! Operating-system entropy for per-shape pixelation seeds.

/// The operating system did not provide the required random bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntropyUnavailable;

/// Fetch a seed for one shape. Failure must cancel creation rather than fall
/// back to a predictable value.
#[cfg(windows)]
pub fn operating_system_seed() -> Result<u64, EntropyUnavailable> {
    use windows::Win32::Security::Cryptography::{
        BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
    };
    let mut bytes = [0_u8; 8];
    // SAFETY CONTRACT: The mutable slice owns eight writable bytes for the
    // duration of this call; the system-preferred provider needs no handle.
    // BCryptGenRandom does not dispatch window messages or callbacks here.
    let status = unsafe { BCryptGenRandom(None, &mut bytes, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
    if status.0 < 0 {
        return Err(EntropyUnavailable);
    }
    Ok(u64::from_le_bytes(bytes))
}

#[cfg(not(windows))]
pub fn operating_system_seed() -> Result<u64, EntropyUnavailable> {
    use std::io::Read;
    let mut bytes = [0_u8; 8];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|_| EntropyUnavailable)?;
    Ok(u64::from_le_bytes(bytes))
}
