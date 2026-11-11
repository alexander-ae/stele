//! Small helpers that would be dependencies in a larger program.

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

/// A random identifier as lowercase hex.
pub fn random_hex(bytes: usize) -> Result<String> {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).context("no source of randomness")?;
    Ok(hex(&buf))
}

pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(DIGITS[(b >> 4) as usize] as char);
        out.push(DIGITS[(b & 0x0f) as usize] as char);
    }
    out
}

/// Tokens are stored as their SHA-256, never in the clear. The token itself is
/// 32 random bytes, so a fast hash is the right choice: there is no password
/// to stretch.
pub fn sha256_hex(value: &str) -> String {
    hex(&Sha256::digest(value.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_encoding() {
        assert_eq!(hex(&[0x00, 0x0f, 0xa5, 0xff]), "000fa5ff");
    }

    #[test]
    fn sha256_is_stable() {
        assert_eq!(
            sha256_hex("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn random_ids_differ() {
        assert_ne!(random_hex(16).unwrap(), random_hex(16).unwrap());
    }
}
