//! HMAC-SHA256 as defined by RFC 2104, built on the existing `sha2` dependency.
//!
//! Correctness is pinned by the RFC 4231 known-answer vectors in the tests.

use sha2::{Digest, Sha256};

/// SHA-256 block size; RFC 2104 pads the key to exactly one block.
const BLOCK: usize = 64;
const IPAD: u8 = 0x36;
const OPAD: u8 = 0x5c;

/// Length of a signature as lower-case hex, its wire form.
pub const MAC_HEX_LEN: usize = 64;

pub fn sign(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut padded = [0_u8; BLOCK];
    // RFC 2104: a key longer than the block is replaced by its digest; a shorter one is zero-padded.
    if key.len() > BLOCK {
        padded[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        padded[..key.len()].copy_from_slice(key);
    }

    let mut inner_pad = [0_u8; BLOCK];
    let mut outer_pad = [0_u8; BLOCK];
    for (index, byte) in padded.iter().enumerate() {
        inner_pad[index] = byte ^ IPAD;
        outer_pad[index] = byte ^ OPAD;
    }

    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(message);
    let inner = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner);
    let signature = outer.finalize();

    // Best-effort wipe of key material; the compiler may elide these writes.
    padded.fill(0);
    inner_pad.fill(0);
    outer_pad.fill(0);

    signature.into()
}

pub fn sign_hex(key: &[u8], message: &[u8]) -> String {
    sign(key, message)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Whether `presented` is the signature of `message`, compared in constant time.
///
/// An early-exit comparison would let a sender recover a valid signature byte by byte by timing.
/// Malformed or wrong-length hex is refused up front; a length is not a secret.
pub fn verify_hex(key: &[u8], message: &[u8], presented: &str) -> bool {
    let Some(presented) = decode(presented) else {
        return false;
    };
    let expected = sign(key, message);
    let mut difference = 0_u8;
    for (left, right) in expected.iter().zip(presented.iter()) {
        difference |= left ^ right;
    }
    difference == 0
}

fn decode(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != MAC_HEX_LEN {
        return None;
    }
    let bytes = hex.as_bytes();
    let mut out = [0_u8; 32];
    for (index, slot) in out.iter_mut().enumerate() {
        let high = digit(bytes[index * 2])?;
        let low = digit(bytes[index * 2 + 1])?;
        *slot = (high << 4) | low;
    }
    Some(out)
}

/// Accepts both cases: the wire format is lower-case, but an upper-case signature is not a forgery.
fn digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 4231 test cases 1-7 (except the truncation case), with the published answers.
    #[test]
    fn the_rfc_4231_vectors_come_out_right() {
        let cases: [(Vec<u8>, Vec<u8>, &str); 6] = [
            (
                vec![0x0b; 20],
                b"Hi There".to_vec(),
                "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7",
            ),
            (
                b"Jefe".to_vec(),
                b"what do ya want for nothing?".to_vec(),
                "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
            ),
            (
                vec![0xaa; 20],
                vec![0xdd; 50],
                "773ea91e36800e46854db8ebd09181a72959098b3ef8c122d9635514ced565fe",
            ),
            (
                (1..=25).collect(),
                vec![0xcd; 50],
                "82558a389a443c0ea4cc819899f2083a85f0faa3e578f8077a2e3ff46729665b",
            ),
            (
                vec![0xaa; 131],
                b"Test Using Larger Than Block-Size Key - Hash Key First".to_vec(),
                "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54",
            ),
            (
                vec![0xaa; 131],
                b"This is a test using a larger than block-size key and a larger \
                  than block-size data. The key needs to be hashed before being \
                  used by the HMAC algorithm."
                    .to_vec(),
                "9b09ffa71b942fcb27635fbcd5b0e944bfdc63644f0713938a7f51535c3a35e2",
            ),
        ];

        for (key, message, expected) in cases {
            assert_eq!(sign_hex(&key, &message), expected);
        }
    }

    /// RFC 4231 test case 5 is the truncation case; only the leading 128 bits are published.
    #[test]
    fn the_truncation_vector_matches_on_its_published_prefix() {
        let signature = sign_hex(&[0x0c; 20], b"Test With Truncation");
        assert_eq!(&signature[..32], "a3b6167473100ee06e0c796c2955552b");
    }

    /// Refusing an empty key belongs to the settings boundary, not to this function.
    #[test]
    fn an_empty_key_is_padded_rather_than_refused() {
        assert_eq!(
            sign_hex(b"", b""),
            "b613679a0814d9ec772f95d778c35fc5ff1697c493715653c6c712144292c5ad"
        );
    }

    /// A key of exactly one block must be padded, not hashed first.
    #[test]
    fn a_key_of_exactly_one_block_is_not_hashed_first() {
        let exact = vec![0xaa; BLOCK];
        let longer = vec![0xaa; BLOCK + 1];
        assert_ne!(sign_hex(&exact, b"x"), sign_hex(&longer, b"x"));
        assert_eq!(sign_hex(&exact, b"x"), sign_hex(&exact, b"x"));
    }

    #[test]
    fn a_signature_verifies_against_its_own_message_and_key() {
        let key = b"shared-secret";
        let message = b"toglet-remote/1\ncommand\nresume";
        let signature = sign_hex(key, message);
        assert!(verify_hex(key, message, &signature));
    }

    #[test]
    fn changing_one_byte_anywhere_refuses_the_signature() {
        let key = b"shared-secret";
        let message = b"toglet-remote/1\ncommand\nresume";
        let signature = sign_hex(key, message);

        let mut tampered = signature.clone();
        tampered.replace_range(0..1, if signature.starts_with('a') { "b" } else { "a" });
        assert!(!verify_hex(key, message, &tampered));

        assert!(!verify_hex(
            key,
            b"toglet-remote/1\ncommand\ncancel",
            &signature
        ));
        assert!(!verify_hex(b"another-secret", message, &signature));
    }

    #[test]
    fn a_signature_that_is_not_hex_of_the_right_length_is_refused() {
        let key = b"shared-secret";
        let message = b"x";
        let good = sign_hex(key, message);

        assert!(!verify_hex(key, message, ""));
        assert!(!verify_hex(key, message, &good[..MAC_HEX_LEN - 1]));
        assert!(!verify_hex(key, message, &format!("{good}0")));
        assert!(!verify_hex(key, message, &"z".repeat(MAC_HEX_LEN)));
    }

    #[test]
    fn case_does_not_decide_whether_a_signature_is_genuine() {
        let key = b"shared-secret";
        let message = b"x";
        let signature = sign_hex(key, message);
        assert!(verify_hex(key, message, &signature.to_uppercase()));
    }

    #[test]
    fn a_signature_is_always_sixty_four_lower_case_hex_characters() {
        let signature = sign_hex(b"k", b"m");
        assert_eq!(signature.len(), MAC_HEX_LEN);
        assert!(
            signature
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
        );
    }
}
