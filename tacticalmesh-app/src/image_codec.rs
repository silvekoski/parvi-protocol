use image::{GrayImage, ImageOutputFormat};

/// Encode raw greyscale bytes as JPEG. Returns `None` if dimensions don't match `raw.len()`.
pub fn encode_jpeg(raw: &[u8], width: u32, height: u32, quality: u8) -> Option<Vec<u8>> {
    let img = GrayImage::from_raw(width, height, raw.to_vec())?;
    let mut buf = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut buf), ImageOutputFormat::Jpeg(quality))
        .ok()?;
    Some(buf)
}

/// Decode JPEG bytes into `(raw_luma8_pixels, width, height)`.
pub fn decode_jpeg(bytes: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let img = image::load_from_memory(bytes).ok()?;
    let gray = img.into_luma8();
    let w = gray.width();
    let h = gray.height();
    Some((gray.into_raw(), w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checkerboard(w: u32, h: u32) -> Vec<u8> {
        (0..h)
            .flat_map(|y| (0..w).map(move |x| if (x + y) % 2 == 0 { 255u8 } else { 0u8 }))
            .collect()
    }

    #[test]
    fn encode_produces_smaller_output() {
        let raw = checkerboard(640, 480);
        let jpeg = encode_jpeg(&raw, 640, 480, 75).expect("encode failed");
        assert!(
            jpeg.len() < raw.len(),
            "JPEG ({} B) should be smaller than raw ({} B)",
            jpeg.len(),
            raw.len()
        );
    }

    #[test]
    fn roundtrip_preserves_dimensions() {
        let raw = checkerboard(320, 240);
        let jpeg = encode_jpeg(&raw, 320, 240, 85).expect("encode failed");
        let (_, w, h) = decode_jpeg(&jpeg).expect("decode failed");
        assert_eq!((w, h), (320, 240));
    }

    #[test]
    fn roundtrip_pixel_values_approximately_correct() {
        // Use a flat grey image so JPEG loss doesn't matter.
        let raw = vec![128u8; 64 * 64];
        let jpeg = encode_jpeg(&raw, 64, 64, 95).expect("encode failed");
        let (pixels, w, h) = decode_jpeg(&jpeg).expect("decode failed");
        assert_eq!((w, h), (64, 64));
        // Every decoded pixel should be within ±10 of the original 128.
        for &p in &pixels {
            assert!((p as i16 - 128).abs() <= 10, "pixel {p} too far from 128");
        }
    }

    #[test]
    fn decode_garbage_returns_none() {
        assert!(decode_jpeg(b"not a jpeg").is_none());
        assert!(decode_jpeg(&[]).is_none());
    }

    #[test]
    fn encode_wrong_size_returns_none() {
        // 10 bytes cannot be a 640×480 image.
        assert!(encode_jpeg(&[0u8; 10], 640, 480, 75).is_none());
    }

    /// Simulates the full shard pipeline: encode → chunk → reassemble → decode.
    #[test]
    fn shard_pipeline_roundtrip() {
        use crate::image_cache::ImageCache;
        use crate::messages::ImageShard;

        const W: u32 = 64;
        const H: u32 = 64;
        let raw = checkerboard(W, H);

        let jpeg = encode_jpeg(&raw, W, H, 85).expect("encode");
        assert!(!jpeg.is_empty());

        const SHARD_SIZE: usize = 200;
        let chunks: Vec<Vec<u8>> = jpeg.chunks(SHARD_SIZE).map(|c| c.to_vec()).collect();
        let total_blocks = chunks.len() as u8;

        let mut cache = ImageCache::new();
        let target_id = 42u16;

        for (block_id, data) in chunks.iter().enumerate() {
            let completed = cache.insert_shard(ImageShard {
                target_id,
                total_blocks,
                block_id: block_id as u8,
                index: 0,
                k: 1,
                n: 1,
                data: data.clone(),
            });
            if block_id as u8 + 1 < total_blocks {
                assert!(!completed, "should not complete before last shard");
            }
        }

        let assembled = cache.get_complete(target_id).expect("cache should be complete");
        let (pixels, w, h) = decode_jpeg(assembled).expect("decode assembled JPEG");
        assert_eq!((w, h), (W, H));
        assert_eq!(pixels.len(), (W * H) as usize);
    }
}
