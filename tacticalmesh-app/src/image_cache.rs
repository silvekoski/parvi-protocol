use std::collections::HashMap;

use crate::messages::ImageShard;

pub struct ImageCache {
    /// shards[(target_id, block_id)] = vec of (index, data)
    shards: HashMap<(u16, u8), Vec<(u8, Vec<u8>)>>,
    /// k per block
    block_k: HashMap<(u16, u8), u8>,
    /// assembled data per (target_id, block_id)
    assembled_blocks: HashMap<(u16, u8), Vec<u8>>,
    /// total blocks expected per target (0/1 = single-block legacy mode)
    total_blocks_expected: HashMap<u16, u8>,
    /// fully assembled images
    complete: HashMap<u16, Vec<u8>>,
}

impl ImageCache {
    pub fn new() -> Self {
        Self {
            shards: HashMap::new(),
            block_k: HashMap::new(),
            assembled_blocks: HashMap::new(),
            total_blocks_expected: HashMap::new(),
            complete: HashMap::new(),
        }
    }

    /// Insert a shard. Returns true if the image for `target_id` is now complete.
    pub fn insert_shard(&mut self, shard: ImageShard) -> bool {
        let key = (shard.target_id, shard.block_id);
        let target_id = shard.target_id;
        let block_id = shard.block_id;
        let k = shard.k;
        let total = shard.total_blocks;
        let shard_index = shard.index;

        if total > 1 {
            self.total_blocks_expected.entry(target_id).or_insert(total);
        }
        self.block_k.entry(key).or_insert(k);

        {
            let bucket = self.shards.entry(key).or_insert_with(Vec::new);
            if !bucket.iter().any(|(idx, _)| *idx == shard_index) {
                bucket.push((shard_index, shard.data));
            }
        }

        // Try to assemble this FEC block.
        let threshold = *self.block_k.get(&key).unwrap_or(&k) as usize;
        let bucket_len = self.shards.get(&key).map(|b| b.len()).unwrap_or(0);

        if bucket_len >= threshold && !self.assembled_blocks.contains_key(&key) {
            let mut sorted = self.shards.get(&key).unwrap().clone();
            sorted.sort_by_key(|(idx, _)| *idx);
            let assembled: Vec<u8> = sorted.into_iter().flat_map(|(_, data)| data).collect();
            self.assembled_blocks.insert(key, assembled);
        }

        // Try to complete the full image.
        if self.complete.contains_key(&target_id) {
            return false; // already complete
        }

        let expected = *self.total_blocks_expected.get(&target_id).unwrap_or(&0);

        if expected <= 1 {
            // Single-block mode: complete as soon as this block assembles.
            if self.assembled_blocks.contains_key(&(target_id, block_id)) {
                let data = self.assembled_blocks[&(target_id, block_id)].clone();
                self.complete.insert(target_id, data);
                return true;
            }
        } else {
            // Multi-block mode: wait for all blocks.
            let all_done = (0..expected).all(|bid| {
                self.assembled_blocks.contains_key(&(target_id, bid))
            });
            if all_done {
                let mut full: Vec<u8> = Vec::new();
                for bid in 0..expected {
                    if let Some(data) = self.assembled_blocks.get(&(target_id, bid)) {
                        full.extend_from_slice(data);
                    }
                }
                self.complete.insert(target_id, full);
                return true;
            }
        }

        false
    }

    pub fn get_complete(&self, target_id: u16) -> Option<&[u8]> {
        self.complete.get(&target_id).map(|v| v.as_slice())
    }

    /// Count how many FEC blocks are fully assembled for `target_id`.
    pub fn blocks_assembled(&self, target_id: u16) -> u8 {
        let expected = *self.total_blocks_expected.get(&target_id).unwrap_or(&1);
        (0..expected)
            .filter(|&bid| self.assembled_blocks.contains_key(&(target_id, bid)))
            .count() as u8
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
            total_blocks: 0,
            block_id,
            index,
            k,
            n,
            data: vec![index; 16],
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
        assert_eq!(result.unwrap().len(), 8 * 16);
    }

    #[test]
    fn seven_shards_does_not_complete_image() {
        let mut cache = ImageCache::new();
        let target_id = 20u16;

        for i in 0..7u8 {
            cache.insert_shard(make_shard(target_id, 0, i, 8, 12));
        }

        assert!(cache.get_complete(target_id).is_none());
    }

    #[test]
    fn has_shard_reflects_insertion() {
        let mut cache = ImageCache::new();
        let target_id = 30u16;

        assert!(!cache.has_shard(target_id, 0));
        cache.insert_shard(make_shard(target_id, 0, 0, 8, 12));
        assert!(cache.has_shard(target_id, 0));
        assert!(!cache.has_shard(target_id, 1));
    }

    #[test]
    fn shards_concatenated_in_index_order() {
        let mut cache = ImageCache::new();
        let target_id = 40u16;

        for i in [3u8, 0, 2, 1, 5, 4, 7, 6].iter().copied() {
            cache.insert_shard(make_shard(target_id, 0, i, 8, 12));
        }

        let result = cache.get_complete(target_id).expect("should be complete");
        for i in 0u8..8 {
            let offset = i as usize * 16;
            assert!(result[offset..offset + 16].iter().all(|&b| b == i));
        }
    }

    #[test]
    fn duplicate_shard_indices_not_double_counted() {
        let mut cache = ImageCache::new();
        let target_id = 50u16;

        cache.insert_shard(make_shard(target_id, 0, 0, 8, 12));
        cache.insert_shard(make_shard(target_id, 0, 0, 8, 12));
        assert!(cache.get_complete(target_id).is_none());
    }

    #[test]
    fn multi_block_requires_all_blocks() {
        let mut cache = ImageCache::new();
        let target_id = 60u16;

        // 3-block image, k=1 n=1 per block
        for bid in 0u8..3 {
            let mut shard = make_shard(target_id, bid, 0, 1, 1);
            shard.total_blocks = 3;
            shard.data = vec![bid; 8];
            cache.insert_shard(shard);
        }

        let result = cache.get_complete(target_id).expect("all blocks present");
        assert_eq!(result.len(), 3 * 8);
        // Block 0 data = [0;8], block 1 = [1;8], block 2 = [2;8]
        assert!(result[0..8].iter().all(|&b| b == 0));
        assert!(result[8..16].iter().all(|&b| b == 1));
        assert!(result[16..24].iter().all(|&b| b == 2));
    }

    #[test]
    fn multi_block_incomplete_without_all_blocks() {
        let mut cache = ImageCache::new();
        let target_id = 70u16;

        // Only 2 of 3 blocks
        for bid in 0u8..2 {
            let mut shard = make_shard(target_id, bid, 0, 1, 1);
            shard.total_blocks = 3;
            cache.insert_shard(shard);
        }

        assert!(cache.get_complete(target_id).is_none());
    }
}
