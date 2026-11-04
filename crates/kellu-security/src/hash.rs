//! SHA-256 helpers shared by grants, proposals and the audit chain.

use sha2::{Digest, Sha256};

/// Length of a SHA-256 digest written in hexadecimal.
pub const SHA256_HEX_LEN: usize = 64;

/// The SHA-256 of `data` as 64 lowercase hexadecimal characters.
#[must_use]
pub fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    digest
        .iter()
        .fold(String::with_capacity(SHA256_HEX_LEN), |mut out, byte| {
            out.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
            out.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
            out
        })
}

/// Whether `text` is a well-formed lowercase SHA-256 hex digest.
#[must_use]
pub fn is_sha256_hex(text: &str) -> bool {
    text.len() == SHA256_HEX_LEN
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn digests_are_recognised() {
        assert!(is_sha256_hex(&sha256_hex(b"x")));
        for bad in ["", "ABC", &"g".repeat(64), &"a".repeat(63), &"A".repeat(64)] {
            assert!(!is_sha256_hex(bad), "{bad}");
        }
    }
}
