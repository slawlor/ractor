// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Factory's defaulting routing hash mechanism. Hashes a [super::JobKey] to a finite
//! space

use std::collections::hash_map::DefaultHasher;
use std::hash::Hash;
use std::hash::Hasher;

/// Hash a key into a finite space
pub fn hash_with_max<TKey>(key: &TKey, excluded_max: usize) -> usize
where
    TKey: Hash,
{
    let mut dh = DefaultHasher::new();
    key.hash(&mut dh);
    let bounded_hash = dh.finish() % (excluded_max as u64);
    bounded_hash as usize
}

#[cfg(test)]
mod tests {

    use super::*;

    #[derive(Hash)]
    struct TestHashable(u128);

    #[test]
    fn test_bounded_hashing() {
        let test_values = [
            TestHashable(0),
            TestHashable(10),
            TestHashable(23),
            TestHashable(128),
            TestHashable(u64::MAX as u128),
        ];

        let test_results = [0usize, 2, 1, 0, 0];

        for (test_value, test_result) in test_values.iter().zip(test_results) {
            let hashed = hash_with_max(test_value, 3);
            assert_eq!(test_result, hashed);
        }
    }
}
