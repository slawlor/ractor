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

#[cfg(test)]
mod tests {

    use super::challenge_digest;
    use super::DIGEST_BYTES;

    #[test]
    fn test_challenge_digest_generation() {
        let secret = "cookie";
        let challenge: u32 = 42;
        let digest = challenge_digest(secret, challenge);
        assert_eq!(DIGEST_BYTES, digest.len());
        assert_eq!(
            digest,
            [
                20, 62, 0, 217, 211, 179, 29, 157, 36, 69, 47, 133, 172, 4, 68, 137, 83, 8, 26, 2,
                237, 2, 39, 46, 89, 44, 91, 19, 205, 66, 46, 247
            ]
        );
    }
}
