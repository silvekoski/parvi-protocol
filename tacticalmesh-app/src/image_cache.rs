use std::collections::HashMap;

use crate::messages::ImageShard;

pub struct ImageCache {
    /// shards[(target_id, block_id)] = vec of (index, data)
    shards: HashMap<(u16, u8), Vec<(u8, Vec<u8>)>>,
    /// assembled images
    complete: HashMap<u16, Vec<u8>>,
    /// k per block
    block_k: HashMap<(u16, u8), u8>,
}

impl ImageCache {
    pub fn new() -> Self {
        Self {
            shards: HashMap::new(),
            complete: HashMap::new(),
            block_k: HashMap::new(),
        }
    }

    /// Store the shard. If k or more shards are present for the block,
    /// concatenate all shard data in index order and mark the image complete.
    pub fn insert_shard(&mut self, shard: ImageShard) {
        let key = (shard.target_id, shard.block_id);
        let k = shard.k;
        let target_id = shard.target_id;
        let shard_index = shard.index;

        self.block_k.entry(key).or_insert(k);

        // Insert shard, avoiding duplicate indices.
        {
            let bucket = self.shards.entry(key).or_insert_with(Vec::new);
            if !bucket.iter().any(|(idx, _)| *idx == shard_index) {
                bucket.push((shard_index, shard.data));
            }
        }

        // Check if we now have enough shards to assemble the block.
        let threshold = *self.block_k.get(&key).unwrap_or(&k) as usize;
        let bucket_len = self.shards.get(&key).map(|b| b.len()).unwrap_or(0);

        if bucket_len >= threshold && !self.complete.contains_key(&target_id) {
            // Sort by index and concatenate — clone to avoid long borrow.
            let mut sorted = self.shards.get(&key).unwrap().clone();
            sorted.sort_by_key(|(idx, _)| *idx);
            let assembled: Vec<u8> = sorted.into_iter().flat_map(|(_, data)| data).collect();
            self.complete.insert(target_id, assembled);
        }
    }

    pub fn get_complete(&self, target_id: u16) -> Option<&[u8]> {
        self.complete.get(&target_id).map(|v| v.as_slice())
    }

    pub fn has_shard(&self, target_id: u16, fec_block: u8) -> bool {
        self.shards
            .get(&(target_id, fec_block))
            .map(|v| !v.is_empty())
            .unwrap_or(false)
    }
}

impl Default for ImageCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_shard(target_id: u16, block_id: u8, index: u8, k: u8, n: u8) -> ImageShard {
        ImageShard {
            target_id,
            block_id,
            index,
            k,
            n,
            data: vec![index; 16], // 16 bytes per shard, value = index for easy verification
        }
    }

    #[test]
    fn eight_shards_completes_image() {
        let mut cache = ImageCache::new();
        let target_id = 10u16;

        for i in 0..8u8 {
            cache.insert_shard(make_shard(target_id, 0, i, 8, 12));
        }

        let result = cache.get_complete(target_id);
        assert!(result.is_some(), "expected image to be complete after k shards");
        // 8 shards * 16 bytes each
        assert_eq!(result.unwrap().len(), 8 * 16);
    }

    #[test]
    fn seven_shards_does_not_complete_image() {
        let mut cache = ImageCache::new();
        let target_id = 20u16;

        for i in 0..7u8 {
            cache.insert_shard(make_shard(target_id, 0, i, 8, 12));
        }

        let result = cache.get_complete(target_id);
        assert!(result.is_none(), "expected image to be incomplete with only 7 shards");
    }

    #[test]
    fn has_shard_reflects_insertion() {
        let mut cache = ImageCache::new();
        let target_id = 30u16;

        assert!(!cache.has_shard(target_id, 0));
        cache.insert_shard(make_shard(target_id, 0, 0, 8, 12));
        assert!(cache.has_shard(target_id, 0));
        // Different block still returns false
        assert!(!cache.has_shard(target_id, 1));
    }

    #[test]
    fn shards_concatenated_in_index_order() {
        let mut cache = ImageCache::new();
        let target_id = 40u16;

        // Insert shards out of order
        for i in [3u8, 0, 2, 1, 5, 4, 7, 6].iter().copied() {
            cache.insert_shard(make_shard(target_id, 0, i, 8, 12));
        }

        let result = cache.get_complete(target_id).expect("should be complete");
        // Each shard's data is vec![index; 16], so result should be:
        // 16 bytes of 0x00, then 16 bytes of 0x01, ..., up to 16 bytes of 0x07
        for i in 0u8..8 {
            let offset = i as usize * 16;
            assert!(
                result[offset..offset + 16].iter().all(|&b| b == i),
                "shard {} data mismatch at offset {}",
                i,
                offset
            );
        }
    }

    #[test]
    fn duplicate_shard_indices_not_double_counted() {
        let mut cache = ImageCache::new();
        let target_id = 50u16;

        // Insert shard index 0 twice — should not count as 2 shards
        cache.insert_shard(make_shard(target_id, 0, 0, 8, 12));
        cache.insert_shard(make_shard(target_id, 0, 0, 8, 12));
        // Only 1 unique shard — not complete
        assert!(cache.get_complete(target_id).is_none());
    }
}
