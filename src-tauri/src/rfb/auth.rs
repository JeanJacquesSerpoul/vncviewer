//! VNC authentication (security type 2): DES challenge/response.

use des::Des;
use des::cipher::generic_array::GenericArray;
use des::cipher::{BlockEncrypt, KeyInit};

/// Builds the DES key from a password: first 8 bytes, zero padded, with the
/// bits of every byte mirrored (a historical quirk of the VNC reference code).
pub fn vnc_des_key(password: &str) -> [u8; 8] {
    let mut key = [0u8; 8];
    for (k, b) in key.iter_mut().zip(password.bytes()) {
        *k = b.reverse_bits();
    }
    key
}

/// Encrypts the 16-byte server challenge with the password.
pub fn challenge_response(password: &str, challenge: &[u8; 16]) -> [u8; 16] {
    des_ecb(&vnc_des_key(password), challenge)
}

fn des_ecb(key: &[u8; 8], data: &[u8; 16]) -> [u8; 16] {
    let cipher = Des::new_from_slice(key).expect("DES key is 8 bytes");
    let mut out = *data;
    for block in out.as_chunks_mut::<8>().0 {
        cipher.encrypt_block(GenericArray::from_mut_slice(block));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_bits_are_mirrored_and_padded() {
        assert_eq!(vnc_des_key("a\x01"), [0x86, 0x80, 0, 0, 0, 0, 0, 0]);
        // Only the first 8 characters are significant.
        assert_eq!(vnc_des_key("123456789"), vnc_des_key("12345678"));
    }

    #[test]
    fn des_matches_reference_vector() {
        // Classic FIPS 46 example: key 133457799BBCDFF1, 0123456789ABCDEF -> 85E813540F0AB405.
        let key = [0x13, 0x34, 0x57, 0x79, 0x9B, 0xBC, 0xDF, 0xF1];
        let block = [0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF];
        let mut data = [0u8; 16];
        data[..8].copy_from_slice(&block);
        data[8..].copy_from_slice(&block);
        let out = des_ecb(&key, &data);
        let expected = [0x85, 0xE8, 0x13, 0x54, 0x0F, 0x0A, 0xB4, 0x05];
        assert_eq!(out[..8], expected);
        assert_eq!(out[8..], expected);
    }
}
