// tdata parser - extracts auth_key from Telegram Desktop data folder
// format: key_data file -> localKey -> account mtp data -> auth_key

use md5::Md5;
use rand::Rng;
use sha1::{Digest, Sha1};
use sha2::Sha512;
use std::fs;
use std::path::Path;

use crate::i18n::t;
use crate::mtproto::crypto::{ige_decrypt, ige_encrypt};

const TDF_MAGIC: &[u8; 4] = b"TDF$";

#[derive(Debug, Clone)]
pub struct TDataAccount {
    pub dc_id: i32,
    pub user_id: i64,
    pub auth_key: Vec<u8>,
}

// main entry point: parse tdata folder and extract account info
pub fn parse_tdata(tdata_path: &Path) -> Result<Vec<TDataAccount>, String> {
    dbg_log!("tdata::parse_tdata path={:?}", tdata_path);

    // find the actual tdata subfolder
    let tdata_dir = if tdata_path.join("tdata").exists() {
        tdata_path.join("tdata")
    } else if tdata_path.join("key_datas").exists()
        || tdata_path.join("key_data1").exists()
        || tdata_path.join("key_data0").exists()
    {
        tdata_path.to_path_buf()
    } else {
        // search one level deep
        let mut found = None;
        if let Ok(entries) = fs::read_dir(tdata_path) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() && p.join("key_datas").exists() {
                    found = Some(p);
                    break;
                }
            }
        }
        found.ok_or_else(|| "tdata folder not found".to_string())?
    };

    dbg_log!("tdata::parse_tdata using dir={:?}", tdata_dir);

    // step 1: read key_data file
    let key_data = read_tdf_file(&tdata_dir, "key_data")?;
    dbg_log!(
        "tdata::parse_tdata key_data read OK, {} bytes",
        key_data.len()
    );

    // step 2: parse salt + keyEncrypted + infoEncrypted from key_data
    let mut pos = 0;
    let salt = read_qbytearray(&key_data, &mut pos)?;
    let key_encrypted = read_qbytearray(&key_data, &mut pos)?;
    let info_encrypted = read_qbytearray(&key_data, &mut pos)?;

    dbg_log!(
        "tdata::parse_tdata salt={} bytes, key_encrypted={} bytes, info_encrypted={} bytes",
        salt.len(),
        key_encrypted.len(),
        info_encrypted.len()
    );

    // step 3: derive passcodeKey from salt (no passcode = empty)
    let passcode_key = create_local_key(&salt, b"");
    dbg_log!("tdata::parse_tdata passcodeKey derived");

    // step 4: decrypt keyEncrypted to get localKey
    let local_key_data = match decrypt_local(&key_encrypted, &passcode_key) {
        Ok(d) => d,
        Err(e) if e.contains("decrypt verification failed") => {
            return Err("local_passcode".to_string());
        }
        Err(e) => return Err(e),
    };
    if local_key_data.len() < 256 {
        return Err(format!(
            "localKey too short: {} bytes",
            local_key_data.len()
        ));
    }
    let local_key: [u8; 256] = local_key_data[..256].try_into().unwrap();
    dbg_log!("tdata::parse_tdata localKey decrypted OK");

    // step 5: decrypt infoEncrypted to get account indices
    let info_data = decrypt_local(&info_encrypted, &local_key)?;
    if info_data.len() < 4 {
        return Err("info data too short".to_string());
    }
    let account_count =
        u32::from_be_bytes([info_data[0], info_data[1], info_data[2], info_data[3]]) as usize;
    dbg_log!("tdata::parse_tdata account_count={}", account_count);

    if account_count == 0 || account_count > 10 {
        return Err(format!("invalid account count: {}", account_count));
    }

    let mut accounts = Vec::new();

    for i in 0..account_count {
        let offset = 4 + i * 4;
        if offset + 4 > info_data.len() {
            break;
        }
        let index = i32::from_be_bytes([
            info_data[offset],
            info_data[offset + 1],
            info_data[offset + 2],
            info_data[offset + 3],
        ]);
        dbg_log!("tdata::parse_tdata account[{}] index={}", i, index);

        // step 6: compute data name key
        let data_name = if index == 0 {
            "data".to_string()
        } else {
            format!("data#{}", index + 1)
        };
        let data_name_key = compute_data_name_key(&data_name);
        let file_part = to_file_part(data_name_key);
        dbg_log!(
            "tdata::parse_tdata data_name='{}' file_part='{}'",
            data_name,
            file_part
        );

        // step 7: read mtp data file
        let mtp_file_path = tdata_dir.join(&file_part);
        let mtp_data = match read_tdf_file_from_dir(&mtp_file_path, &to_file_part(data_name_key)) {
            Ok(d) => d,
            Err(_) => {
                // try reading from tdata_dir directly
                match read_tdf_file(&tdata_dir, &file_part) {
                    Ok(d) => d,
                    Err(e) => {
                        dbg_log!("tdata::parse_tdata SKIP account[{}]: {}", i, e);
                        continue;
                    }
                }
            }
        };

        // the mtp data file is encrypted with localKey
        let mut mtp_pos = 0;
        let mtp_encrypted = read_qbytearray(&mtp_data, &mut mtp_pos)?;
        let mtp_decrypted = decrypt_local(&mtp_encrypted, &local_key)?;

        dbg_log!(
            "tdata::parse_tdata mtp_decrypted {} bytes",
            mtp_decrypted.len()
        );

        // step 8: parse mtp authorization
        match parse_mtp_authorization(&mtp_decrypted) {
            Ok(acc) => {
                dbg_log!(
                    "tdata::parse_tdata account[{}] dc_id={} user_id={} auth_key_len={}",
                    i,
                    acc.dc_id,
                    acc.user_id,
                    acc.auth_key.len()
                );
                accounts.push(acc);
            }
            Err(e) => {
                dbg_log!("tdata::parse_tdata account[{}] parse FAILED: {}", i, e);
            }
        }
    }

    dbg_log!(
        "tdata::parse_tdata done, {} accounts extracted",
        accounts.len()
    );
    Ok(accounts)
}

fn parse_mtp_authorization(data: &[u8]) -> Result<TDataAccount, String> {
    // first 4 bytes = block_id (should be 75 = dbi.MtpAuthorization)
    if data.len() < 4 {
        return Err("mtp data too short".into());
    }
    let block_id = i32::from_be_bytes([data[0], data[1], data[2], data[3]]);
    dbg_log!("tdata::parse_mtp_authorization block_id={}", block_id);

    if block_id != 75 {
        return Err(format!("unexpected block_id: {} (expected 75)", block_id));
    }

    // next is QByteArray with serialized mtp auth
    let mut pos = 4;
    let serialized = read_qbytearray(data, &mut pos)?;
    dbg_log!(
        "tdata::parse_mtp_authorization serialized {} bytes",
        serialized.len()
    );

    // parse serialized: userId(i32) + mainDcId(i32) [or wide ids tag]
    if serialized.len() < 8 {
        return Err("serialized mtp auth too short".into());
    }

    let mut spos = 0;
    let first_i32 = read_i32_be(&serialized, &mut spos)?;
    let second_i32 = read_i32_be(&serialized, &mut spos)?;

    let (user_id, main_dc_id) =
        if ((first_i32 as i64) << 32 | (second_i32 as i64 & 0xFFFFFFFF)) == -1i64 {
            // wide ids tag
            let uid = read_u64_be(&serialized, &mut spos)? as i64;
            let dc = read_i32_be(&serialized, &mut spos)?;
            (uid, dc)
        } else {
            (first_i32 as i64, second_i32)
        };

    dbg_log!(
        "tdata::parse_mtp_authorization user_id={} main_dc_id={}",
        user_id,
        main_dc_id
    );

    // read keys count
    let key_count = read_i32_be(&serialized, &mut spos)? as usize;
    dbg_log!("tdata::parse_mtp_authorization key_count={}", key_count);

    let mut auth_key: Option<Vec<u8>> = None;

    for k in 0..key_count {
        let dc_id = read_i32_be(&serialized, &mut spos)?;
        if spos + 256 > serialized.len() {
            return Err(format!("key[{}] truncated", k));
        }
        let key = serialized[spos..spos + 256].to_vec();
        spos += 256;
        dbg_log!("tdata::parse_mtp_authorization key[{}] dc_id={}", k, dc_id);

        if dc_id == main_dc_id {
            auth_key = Some(key);
        }
    }

    let auth_key = auth_key.ok_or_else(|| "main dc auth_key not found".to_string())?;

    Ok(TDataAccount {
        dc_id: main_dc_id,
        user_id,
        auth_key,
    })
}

// PBKDF2-based key derivation (same as Telegram Desktop)
fn create_local_key(salt: &[u8], passcode: &[u8]) -> [u8; 256] {
    let mut hasher = Sha512::new();
    hasher.update(salt);
    hasher.update(passcode);
    hasher.update(salt);
    let hash = hasher.finalize();

    let iterations = if passcode.is_empty() { 1 } else { 100000 };

    let mut key = [0u8; 256];
    pbkdf2_sha512(&hash, salt, iterations, &mut key);
    key
}

fn pbkdf2_sha512(password: &[u8], salt: &[u8], iterations: u32, output: &mut [u8]) {
    use hmac::{Hmac, Mac};
    type HmacSha512 = Hmac<Sha512>;

    let hlen = 64;
    let num_blocks = (output.len() + hlen - 1) / hlen;

    for block_num in 1..=num_blocks {
        let mut mac = <HmacSha512 as Mac>::new_from_slice(password).unwrap();
        mac.update(salt);
        mac.update(&(block_num as u32).to_be_bytes());
        let u = mac.finalize().into_bytes();
        let mut u_vec = u.to_vec();
        let mut result = u_vec.clone();

        for _ in 1..iterations {
            let mut mac = <HmacSha512 as Mac>::new_from_slice(password).unwrap();
            mac.update(&u_vec);
            let new_u = mac.finalize().into_bytes();
            u_vec = new_u.to_vec();
            for (r, x) in result.iter_mut().zip(u_vec.iter()) {
                *r ^= x;
            }
        }

        let start = (block_num - 1) * hlen;
        let end = std::cmp::min(start + hlen, output.len());
        output[start..end].copy_from_slice(&result[..end - start]);
    }
}

// decrypt tdata encrypted block
fn decrypt_local(encrypted: &[u8], key: &[u8; 256]) -> Result<Vec<u8>, String> {
    if encrypted.len() <= 16 || encrypted.len() % 16 != 0 {
        return Err(format!("bad encrypted size: {}", encrypted.len()));
    }

    let encrypted_key = &encrypted[..16]; // first 16 bytes = SHA1 hash prefix
    let encrypted_data = &encrypted[16..];

    // derive AES key/iv using old mtp method
    let (aes_key, aes_iv) = prepare_aes_oldmtp(key, encrypted_key, false);

    let decrypted = ige_decrypt(encrypted_data, &aes_key, &aes_iv);

    // verify: SHA1(decrypted)[:16] should match encrypted_key
    let mut hasher = Sha1::new();
    hasher.update(&decrypted);
    let check_hash = hasher.finalize();
    if &check_hash[..16] != encrypted_key {
        return Err("decrypt verification failed (bad key or corrupted data)".into());
    }

    // first 4 bytes of decrypted = actual data length
    if decrypted.len() < 4 {
        return Err("decrypted data too short".into());
    }
    let data_len =
        u32::from_le_bytes([decrypted[0], decrypted[1], decrypted[2], decrypted[3]]) as usize;
    if data_len > decrypted.len() || data_len < 4 {
        return Err(format!("bad decrypted data length: {}", data_len));
    }

    // skip the 4-byte length prefix
    Ok(decrypted[4..data_len].to_vec())
}

// old mtp AES key derivation (SHA1-based, different from MTProto 2.0)
// for local decryption x=8 (same as server->client in old mtproto)
fn prepare_aes_oldmtp(key: &[u8; 256], msg_key: &[u8], _send: bool) -> ([u8; 32], [u8; 32]) {
    let x = 8usize; // x=8 for decrypt (local data is always "received")

    let mut sha1_a = Sha1::new();
    sha1_a.update(&msg_key[..16]);
    sha1_a.update(&key[x..x + 32]);
    let a = sha1_a.finalize();

    let mut sha1_b = Sha1::new();
    sha1_b.update(&key[x + 32..x + 48]);
    sha1_b.update(&msg_key[..16]);
    sha1_b.update(&key[x + 48..x + 64]);
    let b = sha1_b.finalize();

    let mut sha1_c = Sha1::new();
    sha1_c.update(&key[x + 64..x + 96]);
    sha1_c.update(&msg_key[..16]);
    let c = sha1_c.finalize();

    let mut sha1_d = Sha1::new();
    sha1_d.update(&msg_key[..16]);
    sha1_d.update(&key[x + 96..x + 128]);
    let d = sha1_d.finalize();

    let mut aes_key = [0u8; 32];
    aes_key[0..8].copy_from_slice(&a[0..8]);
    aes_key[8..20].copy_from_slice(&b[8..20]);
    aes_key[20..32].copy_from_slice(&c[4..16]);

    let mut aes_iv = [0u8; 32];
    aes_iv[0..12].copy_from_slice(&a[8..20]);
    aes_iv[12..20].copy_from_slice(&b[0..8]);
    aes_iv[20..24].copy_from_slice(&c[16..20]);
    aes_iv[24..32].copy_from_slice(&d[0..8]);

    (aes_key, aes_iv)
}

// read TDF file (tries suffixes s, 1, 0)
fn read_tdf_file(dir: &Path, name: &str) -> Result<Vec<u8>, String> {
    for suffix in &["s", "1", "0"] {
        let path = dir.join(format!("{}{}", name, suffix));
        if let Ok(content) = fs::read(&path) {
            if let Ok(data) = parse_tdf_content(&content) {
                dbg_log!("tdata::read_tdf_file OK {:?}", path);
                return Ok(data);
            }
        }
    }
    Err(format!("tdf file '{}' not found in {:?}", name, dir))
}

fn read_tdf_file_from_dir(dir: &Path, name: &str) -> Result<Vec<u8>, String> {
    for suffix in &["s", "1", "0"] {
        let path = dir.join(format!("{}{}", name, suffix));
        if let Ok(content) = fs::read(&path) {
            if let Ok(data) = parse_tdf_content(&content) {
                return Ok(data);
            }
        }
    }
    Err(format!("tdf file '{}' not found in {:?}", name, dir))
}

fn parse_tdf_content(content: &[u8]) -> Result<Vec<u8>, String> {
    if content.len() < 24 {
        // magic(4) + version(4) + at least some data + md5(16)
        return Err("file too short".into());
    }

    if &content[..4] != TDF_MAGIC {
        return Err("invalid TDF magic".into());
    }

    let _version = u32::from_le_bytes([content[4], content[5], content[6], content[7]]);
    let data_size = content.len() - 8 - 16; // minus magic+version and md5
    let data = &content[8..8 + data_size];
    let stored_md5 = &content[8 + data_size..];

    // verify md5: data + dataSize(4 LE) + version(4 LE) + magic(4)
    let mut md5_input = Vec::new();
    md5_input.extend_from_slice(data);
    md5_input.extend_from_slice(&(data_size as u32).to_le_bytes());
    md5_input.extend_from_slice(&content[4..8]); // version
    md5_input.extend_from_slice(TDF_MAGIC);

    let computed_md5 = {
        use md5::Digest;
        let mut hasher = Md5::new();
        hasher.update(&md5_input);
        hasher.finalize()
    };
    if computed_md5.as_slice() != stored_md5 {
        return Err("md5 checksum mismatch".into());
    }

    Ok(data.to_vec())
}

// read QByteArray from buffer (Qt serialization: u32 BE length + data)
fn read_qbytearray(data: &[u8], pos: &mut usize) -> Result<Vec<u8>, String> {
    if *pos + 4 > data.len() {
        return Err("truncated qbytearray length".into());
    }
    let len = u32::from_be_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]]);
    *pos += 4;

    if len == 0xFFFFFFFF {
        // null QByteArray
        return Ok(Vec::new());
    }

    let len = len as usize;
    if *pos + len > data.len() {
        return Err(format!(
            "truncated qbytearray data: need {} have {}",
            len,
            data.len() - *pos
        ));
    }

    let result = data[*pos..*pos + len].to_vec();
    *pos += len;
    Ok(result)
}

fn read_i32_be(data: &[u8], pos: &mut usize) -> Result<i32, String> {
    if *pos + 4 > data.len() {
        return Err("truncated i32".into());
    }
    let v = i32::from_be_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]]);
    *pos += 4;
    Ok(v)
}

fn read_u64_be(data: &[u8], pos: &mut usize) -> Result<u64, String> {
    if *pos + 8 > data.len() {
        return Err("truncated u64".into());
    }
    let v = u64::from_be_bytes([
        data[*pos],
        data[*pos + 1],
        data[*pos + 2],
        data[*pos + 3],
        data[*pos + 4],
        data[*pos + 5],
        data[*pos + 6],
        data[*pos + 7],
    ]);
    *pos += 8;
    Ok(v)
}

fn compute_data_name_key(name: &str) -> u128 {
    use md5::Digest;
    let mut hasher = Md5::new();
    hasher.update(name.as_bytes());
    let hash = hasher.finalize();
    u128::from_le_bytes(hash.into())
}

fn to_file_part(val: u128) -> String {
    let mut result = String::with_capacity(16);
    let mut v = val;
    for _ in 0..16 {
        let nibble = (v & 0xF) as u8;
        if nibble < 0x0A {
            result.push((b'0' + nibble) as char);
        } else {
            result.push((b'A' + nibble - 0x0A) as char);
        }
        v >>= 4;
    }
    result
}

// === WRITE TDATA (session -> tdata conversion) ===

const APP_VERSION: u32 = 4009001;

// generate tdata folder from session data
pub fn write_tdata(output_path: &Path, account: &TDataAccount) -> Result<(), String> {
    if account.user_id == 0 {
        return Err(t("converter_tdata_no_userid"));
    }
    dbg_log!(
        "tdata::write_tdata to {:?} user_id={} dc_id={}",
        output_path,
        account.user_id,
        account.dc_id
    );
    fs::create_dir_all(output_path).map_err(|e| format!("mkdir failed: {e}"))?;

    // generate random localKey (256 bytes)
    let mut local_key = [0u8; 256];
    rand::thread_rng().fill(&mut local_key[..]);

    // non-empty salt required (empty = no existing data, Telegram creates new session)
    let mut salt = [0u8; 32];
    rand::thread_rng().fill(&mut salt[..]);

    // derive passcodeKey from salt + empty passcode (no passcode set)
    let passcode_key = create_local_key(&salt, b"");

    // encrypt localKey with passcodeKey
    let key_encrypted = encrypt_local(&local_key, &passcode_key);

    // step 1: write key_datas
    let info_plain = serialize_account_info(account);
    let info_encrypted = encrypt_local(&info_plain, &local_key);

    let mut key_data_content = Vec::new();
    write_qbytearray(&mut key_data_content, &salt);
    write_qbytearray(&mut key_data_content, &key_encrypted);
    write_qbytearray(&mut key_data_content, &info_encrypted);

    write_tdf_file(output_path, "key_data", &key_data_content)?;

    // step 2: write mtp data file at top level (TDesktop reads it from here)
    let data_name = "data";
    let data_name_key = compute_data_name_key(data_name);
    let file_part = to_file_part(data_name_key);

    let mtp_serialized = serialize_mtp_authorization(account);
    let mut mtp_plain = Vec::new();
    mtp_plain.extend_from_slice(&75i32.to_be_bytes());
    write_qbytearray(&mut mtp_plain, &mtp_serialized);

    let mtp_encrypted = encrypt_local(&mtp_plain, &local_key);
    let mut mtp_file_content = Vec::new();
    write_qbytearray(&mut mtp_file_content, &mtp_encrypted);

    write_tdf_file(output_path, &file_part, &mtp_file_content)?;

    // step 3: create account subfolder with empty map
    let account_path = output_path.join(&file_part);
    fs::create_dir_all(&account_path).ok();

    // empty map — TDesktop will populate it on first successful run
    let map_plain: Vec<u8> = vec![0u8; 0];
    let map_encrypted = encrypt_local(&map_plain, &local_key);
    let mut map_content = Vec::new();
    write_qbytearray(&mut map_content, &[]);
    write_qbytearray(&mut map_content, &[]);
    write_qbytearray(&mut map_content, &map_encrypted);

    write_tdf_file(&account_path, "map", &map_content)?;

    dbg_log!("tdata::write_tdata OK");
    Ok(())
}

fn serialize_account_info(_account: &TDataAccount) -> Vec<u8> {
    // info: count(i32 BE) + index(i32 BE) [+ active_index(i32 BE)]
    let mut data = Vec::new();
    data.extend_from_slice(&1i32.to_be_bytes()); // 1 account
    data.extend_from_slice(&0i32.to_be_bytes()); // index 0
    data.extend_from_slice(&0i32.to_be_bytes()); // active index 0
    data
}

fn serialize_mtp_authorization(account: &TDataAccount) -> Vec<u8> {
    // format: wide_ids_tag(i32+i32=-1) + user_id(u64 BE) + dc_id(i32 BE) + key_count(i32 BE) + [dc_id(i32 BE) + key(256)]
    let mut data = Vec::new();

    // wide ids tag: userId(i32) << 32 | mainDcId(i32) == -1 (0xFFFFFFFF_FFFFFFFF)
    data.extend_from_slice(&(-1i32).to_be_bytes()); // first part of tag
    data.extend_from_slice(&(-1i32).to_be_bytes()); // second part of tag

    // actual user_id as u64 BE
    data.extend_from_slice(&(account.user_id as u64).to_be_bytes());
    // main dc_id
    data.extend_from_slice(&account.dc_id.to_be_bytes());

    // keys count = 1
    data.extend_from_slice(&1i32.to_be_bytes());
    // key: dc_id + 256 bytes
    data.extend_from_slice(&account.dc_id.to_be_bytes());
    data.extend_from_slice(&account.auth_key);

    // keys to destroy count = 0
    data.extend_from_slice(&0i32.to_be_bytes());

    data
}

fn encrypt_local(plaintext: &[u8], key: &[u8; 256]) -> Vec<u8> {
    // format: sha1_prefix(16) + aes_ige_encrypted(padded_data)
    // padded_data = length(4 LE) + plaintext + padding_to_16

    let mut to_encrypt = Vec::new();
    let data_len = (plaintext.len() + 4) as u32;
    to_encrypt.extend_from_slice(&data_len.to_le_bytes());
    to_encrypt.extend_from_slice(plaintext);

    // pad to 16 bytes
    let pad_needed = (16 - (to_encrypt.len() % 16)) % 16;
    to_encrypt.extend(std::iter::repeat(0u8).take(pad_needed));

    // sha1 of padded data = encryption key (first 16 bytes)
    let mut hasher = Sha1::new();
    hasher.update(&to_encrypt);
    let hash = hasher.finalize();
    let msg_key: [u8; 16] = hash[..16].try_into().unwrap();

    // derive AES key/iv (x=0 for encrypt in old mtp)
    let (aes_key, aes_iv) = prepare_aes_oldmtp_encrypt(key, &msg_key);

    let encrypted = ige_encrypt(&to_encrypt, &aes_key, &aes_iv);

    let mut result = Vec::with_capacity(16 + encrypted.len());
    result.extend_from_slice(&msg_key);
    result.extend_from_slice(&encrypted);
    result
}

// for encryption of local data x=8 (same as decrypt — local storage uses same key derivation for both)
fn prepare_aes_oldmtp_encrypt(key: &[u8; 256], msg_key: &[u8]) -> ([u8; 32], [u8; 32]) {
    let x = 8usize;

    let mut sha1_a = Sha1::new();
    sha1_a.update(&msg_key[..16]);
    sha1_a.update(&key[x..x + 32]);
    let a = sha1_a.finalize();

    let mut sha1_b = Sha1::new();
    sha1_b.update(&key[x + 32..x + 48]);
    sha1_b.update(&msg_key[..16]);
    sha1_b.update(&key[x + 48..x + 64]);
    let b = sha1_b.finalize();

    let mut sha1_c = Sha1::new();
    sha1_c.update(&key[x + 64..x + 96]);
    sha1_c.update(&msg_key[..16]);
    let c = sha1_c.finalize();

    let mut sha1_d = Sha1::new();
    sha1_d.update(&msg_key[..16]);
    sha1_d.update(&key[x + 96..x + 128]);
    let d = sha1_d.finalize();

    let mut aes_key = [0u8; 32];
    aes_key[0..8].copy_from_slice(&a[0..8]);
    aes_key[8..20].copy_from_slice(&b[8..20]);
    aes_key[20..32].copy_from_slice(&c[4..16]);

    let mut aes_iv = [0u8; 32];
    aes_iv[0..12].copy_from_slice(&a[8..20]);
    aes_iv[12..20].copy_from_slice(&b[0..8]);
    aes_iv[20..24].copy_from_slice(&c[16..20]);
    aes_iv[24..32].copy_from_slice(&d[0..8]);

    (aes_key, aes_iv)
}

fn write_qbytearray(buf: &mut Vec<u8>, data: &[u8]) {
    if data.is_empty() {
        buf.extend_from_slice(&0xFFFFFFFFu32.to_be_bytes()); // null QByteArray
    } else {
        buf.extend_from_slice(&(data.len() as u32).to_be_bytes());
        buf.extend_from_slice(data);
    }
}

fn write_tdf_file(dir: &Path, name: &str, data: &[u8]) -> Result<(), String> {
    let path = dir.join(format!("{}s", name));
    dbg_log!("tdata::write_tdf_file {:?}", path);

    let mut content = Vec::new();
    content.extend_from_slice(TDF_MAGIC);
    content.extend_from_slice(&APP_VERSION.to_le_bytes());
    content.extend_from_slice(data);

    // md5: data + dataSize(4 LE) + version(4 LE) + magic(4)
    let mut md5_input = Vec::new();
    md5_input.extend_from_slice(data);
    md5_input.extend_from_slice(&(data.len() as u32).to_le_bytes());
    md5_input.extend_from_slice(&APP_VERSION.to_le_bytes());
    md5_input.extend_from_slice(TDF_MAGIC);

    let md5_hash = {
        use md5::Digest as _;
        let mut h = Md5::new();
        h.update(&md5_input);
        h.finalize()
    };
    content.extend_from_slice(&md5_hash);

    fs::write(&path, &content).map_err(|e| format!("write failed: {e}"))?;
    Ok(())
}
