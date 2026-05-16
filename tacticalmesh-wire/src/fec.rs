use crate::errors::FrameError;
use crate::priority::Priority;

pub const FEC_THRESHOLD_BYTES: usize = 1024;

/// (data_shards, total_shards) per priority.
pub fn fec_params(prio: Priority) -> (usize, usize) {
    match prio {
        Priority::Emergency => (1, 4),
        Priority::Critical  => (1, 3),
        Priority::High      => (1, 2),
        Priority::Bulk      => (8, 12),
    }
}

/// Encodes `data` into shards. Returns `(data_shards, recovery_shards)`.
/// Each returned shard has the same length (data is zero-padded to align).
pub fn fec_encode(data: &[u8], prio: Priority) -> Result<Vec<Vec<u8>>, FrameError> {
    let (k, n) = fec_params(prio);
    let recovery = n - k;

    // Split data evenly across k data shards (pad last shard with zeros).
    let shard_size = data.len().div_ceil(k);
    let mut padded = data.to_vec();
    padded.resize(shard_size * k, 0);

    let original_shards: Vec<&[u8]> = padded.chunks(shard_size).collect();

    let recovery_shards = reed_solomon_simd::encode(k, recovery, &original_shards)
        .map_err(|e| FrameError::FecError(e.to_string()))?;

    let mut all = Vec::with_capacity(n);
    for s in &original_shards {
        all.push(s.to_vec());
    }
    for s in &recovery_shards {
        all.push(s.clone());
    }
    Ok(all)
}

/// Decodes shards back to the original data (before zero-padding).
/// `shards` has length `n`; missing shards are `None`.
pub fn fec_decode(
    shards: &[Option<Vec<u8>>],
    k: usize,
    n: usize,
    original_len: usize,
) -> Result<Vec<u8>, FrameError> {
    let recovery = n - k;
    let shard_size = shards.iter().flatten().next()
        .map(|s| s.len())
        .ok_or_else(|| FrameError::FecError("all shards missing".into()))?;

    let mut original_provided: Vec<(usize, &[u8])> = Vec::new();
    let mut recovery_provided: Vec<(usize, &[u8])> = Vec::new();

    for (i, shard) in shards.iter().enumerate() {
        if let Some(s) = shard {
            if i < k {
                original_provided.push((i, s.as_slice()));
            } else {
                recovery_provided.push((i - k, s.as_slice()));
            }
        }
    }

    // If all data shards are present we can reconstruct without RS.
    if original_provided.len() == k {
        let mut out: Vec<u8> = original_provided.iter()
            .flat_map(|(_, s)| s.iter().copied())
            .collect();
        out.truncate(original_len);
        return Ok(out);
    }

    let mut decoder = reed_solomon_simd::ReedSolomonDecoder::new(k, recovery, shard_size)
        .map_err(|e| FrameError::FecError(e.to_string()))?;

    for (idx, data) in &original_provided {
        decoder.add_original_shard(*idx, data)
            .map_err(|e| FrameError::FecError(e.to_string()))?;
    }
    for (idx, data) in &recovery_provided {
        decoder.add_recovery_shard(*idx, data)
            .map_err(|e| FrameError::FecError(e.to_string()))?;
    }

    let result = decoder.decode()
        .map_err(|e| FrameError::FecError(e.to_string()))?;

    let mut reconstructed = Vec::with_capacity(k * shard_size);
    for i in 0..k {
        if let Some(s) = shards[i].as_ref() {
            reconstructed.extend_from_slice(s);
        } else {
            let restored = result.restored_original(i)
                .ok_or_else(|| FrameError::FecError(format!("shard {i} not restored")))?;
            reconstructed.extend_from_slice(restored);
        }
    }
    reconstructed.truncate(original_len);
    Ok(reconstructed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p3_encode_and_full_decode() {
        let data: Vec<u8> = (0..2048).map(|i| i as u8).collect();
        let shards = fec_encode(&data, Priority::Bulk).unwrap();
        let (k, n) = fec_params(Priority::Bulk); // (8, 12)
        assert_eq!(shards.len(), n);

        let wrapped: Vec<Option<Vec<u8>>> = shards.into_iter().map(Some).collect();
        let decoded = fec_decode(&wrapped, k, n, data.len()).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn p3_corrupt_two_shards_reconstruct() {
        let data: Vec<u8> = (0..2048).map(|i| i as u8).collect();
        let shards = fec_encode(&data, Priority::Bulk).unwrap();
        let (k, n) = fec_params(Priority::Bulk); // (8, 12), 4 parity → can lose ≤4

        // Drop shards at index 3 and 9 (one data, one recovery)
        let wrapped: Vec<Option<Vec<u8>>> = shards.into_iter().enumerate()
            .map(|(i, s)| if i == 3 || i == 9 { None } else { Some(s) })
            .collect();

        let decoded = fec_decode(&wrapped, k, n, data.len()).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn p0_encode_three_parity_shards() {
        let data: Vec<u8> = vec![0xDE; 2000];
        let shards = fec_encode(&data, Priority::Emergency).unwrap();
        let (k, n) = fec_params(Priority::Emergency); // (1, 4)
        assert_eq!(shards.len(), n);
        assert_eq!(k, 1);
    }
}
