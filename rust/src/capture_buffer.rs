//! Checked allocation of initialized BGRA bytes for native screen capture.

const MAX_CAPTURE_BYTES: usize = 512 * 1024 * 1024;
const ALLOCATION_ERROR: &str = "Nicht genügend Speicher für die Bildschirmaufnahme.";

/// Reserve the complete output before initializing it or passing it to GDI.
/// A successful reservation does not guarantee physical memory under OS overcommit.
pub fn try_bgra_buffer(width: usize, height: usize) -> Result<Vec<u8>, &'static str> {
    let len = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or("Bildgröße überläuft.")?;
    if width == 0 || height == 0 || len > MAX_CAPTURE_BYTES || len > u32::MAX as usize {
        return Err("Aufnahmebereich überschreitet die zulässige Bildgröße.");
    }
    try_zeroed_buffer(len)
}

fn try_zeroed_buffer(len: usize) -> Result<Vec<u8>, &'static str> {
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(len)
        .map_err(|_| ALLOCATION_ERROR)?;
    // The reservation provides capacity for every byte, so resize cannot allocate.
    // Keep the entire output initialized even if native capture writes only part.
    pixels.resize(len, 0);
    Ok(pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_buffer_is_fully_initialized() {
        assert_eq!(try_bgra_buffer(3, 2).unwrap(), vec![0; 24]);
    }

    #[test]
    fn invalid_capture_sizes_are_rejected_before_reservation() {
        assert_eq!(try_bgra_buffer(usize::MAX, 2), Err("Bildgröße überläuft."));
        assert_eq!(
            try_bgra_buffer(usize::MAX / 4 + 1, 1),
            Err("Bildgröße überläuft.")
        );
        assert!(try_bgra_buffer(MAX_CAPTURE_BYTES / 4 + 1, 1).is_err());
        assert!(try_bgra_buffer(0, 1).is_err());
        assert!(try_bgra_buffer(1, 0).is_err());
    }

    #[test]
    fn impossible_vector_capacity_reports_capture_allocation_error() {
        // u8 capacity beyond isize::MAX is rejected by Vec before the allocator
        // is called; this exercises the error path without allocating huge memory.
        assert_eq!(try_zeroed_buffer(usize::MAX), Err(ALLOCATION_ERROR));
    }
}
