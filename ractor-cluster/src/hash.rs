// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Hashing utilities mainly used around challenge computation

pub(crate) const DIGEST_BYTES: usize = 32;
pub(crate) type Digest = [u8; DIGEST_BYTES];

/// Compute a challenge digest
pub(crate) fn challenge_digest(secret: &'_ str, challenge: u32) -> Digest {
    use sha2::Digest;

    let secret_bytes = secret.as_bytes();
    let mut data = vec![0u8; secret_bytes.len() + 4];

    let challenge_bytes = challenge.to_be_bytes();
    data[0..4].copy_from_slice(&challenge_bytes);
    data[4..].copy_from_slice(secret_bytes);

    let hash = sha2::Sha256::digest(&data);

    hash.into()
}
