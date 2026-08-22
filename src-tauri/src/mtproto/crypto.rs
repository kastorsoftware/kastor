use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit};
use aes::Aes256;
use byteorder::{LittleEndian, WriteBytesExt};
use sha2::{Digest, Sha256};
use std::io::Cursor;

// aes-ige-256 encrypt (mtproto wire format)
pub fn ige_encrypt(data: &[u8], key: &[u8; 32], iv: &[u8; 32]) -> Vec<u8> {
    assert!(data.len() % 16 == 0);
    let cipher = Aes256::new(key.into());

    let mut iv1 = [0u8; 16];
    let mut iv2 = [0u8; 16];
    iv1.copy_from_slice(&iv[0..16]);
    iv2.copy_from_slice(&iv[16..32]);

    let mut result = Vec::with_capacity(data.len());

    for chunk in data.chunks(16) {
        let mut block = [0u8; 16];
        for i in 0..16 {
            block[i] = chunk[i] ^ iv1[i];
        }
        cipher.encrypt_block((&mut block).into());
        for i in 0..16 {
            block[i] ^= iv2[i];
        }
        iv1.copy_from_slice(&block);
        iv2.copy_from_slice(chunk);
        result.extend_from_slice(&block);
    }

    result
}

// aes-ige-256 decrypt
pub fn ige_decrypt(data: &[u8], key: &[u8; 32], iv: &[u8; 32]) -> Vec<u8> {
    assert!(data.len() % 16 == 0);
    let cipher = Aes256::new(key.into());

    let mut iv1 = [0u8; 16];
    let mut iv2 = [0u8; 16];
    iv1.copy_from_slice(&iv[0..16]);
    iv2.copy_from_slice(&iv[16..32]);

    let mut result = Vec::with_capacity(data.len());

    for chunk in data.chunks(16) {
        let mut block = [0u8; 16];
        for i in 0..16 {
            block[i] = chunk[i] ^ iv2[i];
        }
        cipher.decrypt_block((&mut block).into());
        for i in 0..16 {
            block[i] ^= iv1[i];
        }
        iv1.copy_from_slice(chunk);
        iv2.copy_from_slice(&block);
        result.extend_from_slice(&block);
    }

    result
}

// msg_key = sha256(auth_key[88+x..120+x] + plaintext)[8..24]
fn calc_msg_key_with_x(auth_key: &[u8; 256], plaintext: &[u8], x: usize) -> [u8; 16] {
    let mut hasher = Sha256::new();
    hasher.update(&auth_key[88 + x..120 + x]);
    hasher.update(plaintext);
    let hash = hasher.finalize();
    let mut msg_key = [0u8; 16];
    msg_key.copy_from_slice(&hash[8..24]);
    msg_key
}

pub fn calc_msg_key(auth_key: &[u8; 256], plaintext: &[u8]) -> [u8; 16] {
    calc_msg_key_with_x(auth_key, plaintext, 0)
}

// kdf for client->server (x=0)
pub fn kdf_client(auth_key: &[u8; 256], msg_key: &[u8; 16]) -> ([u8; 32], [u8; 32]) {
    kdf(auth_key, msg_key, 0)
}

// kdf for server->client (x=8)
pub fn kdf_server(auth_key: &[u8; 256], msg_key: &[u8; 16]) -> ([u8; 32], [u8; 32]) {
    kdf(auth_key, msg_key, 8)
}

fn kdf(auth_key: &[u8; 256], msg_key: &[u8; 16], x: usize) -> ([u8; 32], [u8; 32]) {
    let mut sha_a = Sha256::new();
    sha_a.update(msg_key);
    sha_a.update(&auth_key[x..x + 36]);
    let a = sha_a.finalize();

    let mut sha_b = Sha256::new();
    sha_b.update(&auth_key[x + 40..x + 76]);
    sha_b.update(msg_key);
    let b = sha_b.finalize();

    let mut key = [0u8; 32];
    key[0..8].copy_from_slice(&a[0..8]);
    key[8..24].copy_from_slice(&b[8..24]);
    key[24..32].copy_from_slice(&a[24..32]);

    let mut iv = [0u8; 32];
    iv[0..8].copy_from_slice(&b[0..8]);
    iv[8..24].copy_from_slice(&a[8..24]);
    iv[24..32].copy_from_slice(&b[24..32]);

    (key, iv)
}

// auth_key_id = lower 64 bits of sha1(auth_key)
pub fn auth_key_id(auth_key: &[u8; 256]) -> u64 {
    use sha1::{Digest as _, Sha1};
    let mut hasher = Sha1::new();
    hasher.update(auth_key);
    let hash = hasher.finalize();
    let mut cursor = Cursor::new(&hash[12..20]);
    use byteorder::ReadBytesExt;
    cursor.read_u64::<LittleEndian>().unwrap()
}

// encrypt plaintext for sending (mtproto 2.0)
pub fn encrypt_message(auth_key: &[u8; 256], plaintext: &[u8]) -> Vec<u8> {
    // mtproto 2.0: padding 12..1024 bytes, total divisible by 16
    let padding_needed = (16 - (plaintext.len() % 16)) % 16;
    let total_padding = if padding_needed < 12 {
        padding_needed + 16
    } else {
        padding_needed
    };
    let mut padded = Vec::with_capacity(plaintext.len() + total_padding);
    padded.extend_from_slice(plaintext);
    let mut rng_buf = vec![0u8; total_padding];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut rng_buf);
    padded.extend_from_slice(&rng_buf);

    let msg_key = calc_msg_key(auth_key, &padded);
    let (aes_key, aes_iv) = kdf_client(auth_key, &msg_key);
    let encrypted = ige_encrypt(&padded, &aes_key, &aes_iv);

    let key_id = auth_key_id(auth_key);
    let mut result = Vec::with_capacity(8 + 16 + encrypted.len());
    result.write_u64::<LittleEndian>(key_id).unwrap();
    result.extend_from_slice(&msg_key);
    result.extend_from_slice(&encrypted);
    result
}

// decrypt received message
pub fn decrypt_message(auth_key: &[u8; 256], data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() < 40 || (data.len() - 24) % 16 != 0 {
        return Err("message too short".into());
    }
    let msg_key: [u8; 16] = data[8..24].try_into().unwrap();
    let encrypted = &data[24..];

    let (aes_key, aes_iv) = kdf_server(auth_key, &msg_key);
    let decrypted = ige_decrypt(encrypted, &aes_key, &aes_iv);
    if calc_msg_key_with_x(auth_key, &decrypted, 8) != msg_key {
        return Err("message key mismatch".into());
    }

    Ok(decrypted)
}
