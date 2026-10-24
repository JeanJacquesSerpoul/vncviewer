//! Apple Remote Desktop authentication (security type 30), used by macOS
//! Screen Sharing: Diffie-Hellman key agreement, then the user name and
//! password encrypted with AES-128-ECB under the MD5 of the shared secret.

use aes::Aes128;
use aes::cipher::generic_array::GenericArray;
use aes::cipher::{BlockEncrypt, KeyInit};
use md5::{Digest, Md5};
use num_bigint::BigUint;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::{Error, Result};

const MAX_KEY_LEN: usize = 1024;

pub async fn authenticate<S>(s: &mut S, username: &str, password: &str) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let generator = s.read_u16().await?;
    let len = s.read_u16().await? as usize;
    if len == 0 || len > MAX_KEY_LEN {
        return Err(Error::Malformed(format!("ARD key length {len}")));
    }
    let mut prime = vec![0u8; len];
    s.read_exact(&mut prime).await?;
    let mut peer = vec![0u8; len];
    s.read_exact(&mut peer).await?;

    let mut private = vec![0u8; len];
    random(&mut private)?;
    let (public, secret) = agree(generator, &prime, &peer, &private);
    let credentials = encrypt_credentials(&secret, username, password)?;
    s.write_all(&credentials).await?;
    s.write_all(&public).await?;
    s.flush().await?;
    Ok(())
}

fn random(buf: &mut [u8]) -> Result<()> {
    getrandom::fill(buf).map_err(|e| Error::Malformed(format!("no random source: {e}")))
}

/// Returns our public key and the shared secret, both `prime.len()` bytes.
fn agree(generator: u16, prime: &[u8], peer: &[u8], private: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let p = BigUint::from_bytes_be(prime);
    let x = BigUint::from_bytes_be(private);
    let public = BigUint::from(generator).modpow(&x, &p);
    let secret = BigUint::from_bytes_be(peer).modpow(&x, &p);
    (pad(&public, prime.len()), pad(&secret, prime.len()))
}

/// Big-endian bytes, left-padded with zeros to `len`.
fn pad(n: &BigUint, len: usize) -> Vec<u8> {
    let bytes = n.to_bytes_be();
    let mut out = vec![0u8; len.saturating_sub(bytes.len())];
    out.extend_from_slice(&bytes);
    out
}

/// 64 bytes of user name then 64 bytes of password, each NUL-terminated and
/// padded with random bytes, encrypted with the MD5 of the shared secret.
fn encrypt_credentials(secret: &[u8], username: &str, password: &str) -> Result<[u8; 128]> {
    let mut block = [0u8; 128];
    random(&mut block)?;
    for (field, value) in block
        .as_chunks_mut::<64>()
        .0
        .iter_mut()
        .zip([username, password])
    {
        let value = value.as_bytes();
        let n = value.len().min(63);
        field[..n].copy_from_slice(&value[..n]);
        field[n] = 0;
    }
    let cipher = Aes128::new(&Md5::digest(secret));
    for chunk in block.as_chunks_mut::<16>().0 {
        cipher.encrypt_block(GenericArray::from_mut_slice(chunk));
    }
    Ok(block)
}

#[cfg(test)]
mod tests {
    use aes::cipher::BlockDecrypt;

    use super::*;

    #[tokio::test]
    async fn server_recovers_the_credentials() {
        // A small safe prime is enough to check the arithmetic and the layout.
        let prime: Vec<u8> = vec![0xFF, 0xFF, 0xFF, 0xC5]; // 4294967237
        let server_private = BigUint::from(123_456_789u32);
        let p = BigUint::from_bytes_be(&prime);
        let server_public = pad(&BigUint::from(2u32).modpow(&server_private, &p), 4);

        let (mut client, mut server) = tokio::io::duplex(1024);
        let srv = tokio::spawn(async move {
            server.write_u16(2).await.unwrap();
            server.write_u16(4).await.unwrap();
            server.write_all(&prime).await.unwrap();
            server.write_all(&server_public).await.unwrap();
            let mut creds = [0u8; 128];
            server.read_exact(&mut creds).await.unwrap();
            let mut client_public = [0u8; 4];
            server.read_exact(&mut client_public).await.unwrap();
            let secret = BigUint::from_bytes_be(&client_public).modpow(&server_private, &p);
            let cipher = Aes128::new(&Md5::digest(pad(&secret, 4)));
            for chunk in creds.as_chunks_mut::<16>().0 {
                cipher.decrypt_block(GenericArray::from_mut_slice(chunk));
            }
            creds
        });
        authenticate(&mut client, "alice", "s3cret").await.unwrap();
        let creds = srv.await.unwrap();
        assert_eq!(&creds[..6], b"alice\0");
        assert_eq!(&creds[64..71], b"s3cret\0");
    }
}
