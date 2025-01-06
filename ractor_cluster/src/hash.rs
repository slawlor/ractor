// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Hashing utilities mainly used around challenge computation

pub(crate) const DIGEST_BYTES: usize = 32;
pub(crate) type Digest = [u8; DIGEST_BYTES];

/// Compute a challenge digest
pub(crate) fn challenge_digest(secret: &'_ str, challenge: u32) -> Digest {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;

    let secret_bytes = secret.as_bytes();
    let challenge_bytes = challenge.to_be_bytes();
    let mut mac = HmacSha256::new_from_slice(secret_bytes).expect("HMAC can take key of any size");
    mac.update(&challenge_bytes);

    mac.finalize().into_bytes().into()
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
                22, 118, 211, 15, 245, 52, 29, 205, 92, 234, 12, 239, 207, 66, 244, 233, 70, 84,
                143, 62, 208, 108, 237, 90, 80, 150, 141, 172, 35, 18, 3, 190
            ]
        );
    }
}
