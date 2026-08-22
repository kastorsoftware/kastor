// mtproto auth — DH key exchange + unauthorized message frame
// reference: https://core.telegram.org/mtproto/auth_key

use byteorder::{LittleEndian, WriteBytesExt};
use num_bigint::BigUint;
use rand::RngCore;
use sha1::{Digest as Sha1Digest, Sha1};
use sha2::Sha256;
use std::io::{Cursor, Read};
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

use byteorder::ReadBytesExt;

use super::crypto::{ige_decrypt, ige_encrypt};
use super::tl;
use super::transport::MtpTransport;

// production rsa public keys (n, e) with their telegram fingerprints
// pub keys decoded from telegram desktop / telethon production set
struct RsaKey {
    fingerprint: u64,
    n: BigUint,
    e: BigUint,
}

const PEM_KEYS: &[&str] = &[
    "-----BEGIN RSA PUBLIC KEY-----\n\
MIIBCgKCAQEAruw2yP/BCcsJliRoW5eBVBVle9dtjJw+OYED160Wybum9SXtBBLX\n\
riwt4rROd9csv0t0OHCaTmRqBcQ0J8fxhN6/cpR1GWgOZRUAiQxoMnlt0R93LCX/\n\
j1dnVa/gVbCjdSxpbrfY2g2L4frzjJvdl84Kd9ORYjDEAyFnEA7dD556OptgLQQ2\n\
e2iVNq8NZLYTzLp5YpOdO1doK+ttrltggTCy5SrKeLoCPPbOgGsdxJxyz5KKcZnS\n\
Lj16yE5HvJQn0CNpRdENvRUXe6tBP78O39oJ8BTHp9oIjd6XWXAsp2CvK45Ol8wF\n\
XGF710w9lwCGNbmNxNYhtIkdqfsEcwR5JwIDAQAB\n\
-----END RSA PUBLIC KEY-----",
    "-----BEGIN RSA PUBLIC KEY-----\n\
MIIBCgKCAQEAvfLHfYH2r9R70w8prHblWt/nDkh+XkgpflqQVcnAfSuTtO05lNPs\n\
pQmL8Y2XjVT4t8cT6xAkdgfmmvnvRPOOKPi0OfJXoRVylFzAQG/j83u5K3kRLbae\n\
7fLccVhKZhY46lvsueI1hQdLgNV9n1cQ3TDS2pQOCtovG4eDl9wacrXOJTG2990V\n\
jgnIKNA0UMoP+KF03qzryqIt3oTvZq03DyWdGK+AZjgBLaDKSnC6qD2cFY81UryR\n\
WOab8zKkWAnhw2kFpcqhI0jdV5QaSCExvnsjVaX0Y1N0870931/5Jb9ICe4nweZ9\n\
kSDF/gip3kWLG0o8XQpChDfyvsqB9OLV/wIDAQAB\n\
-----END RSA PUBLIC KEY-----",
    "-----BEGIN RSA PUBLIC KEY-----\n\
MIIBCgKCAQEAs/ditzm+mPND6xkhzwFIz6J/968CtkcSE/7Z2qAJiXbmZ3UDJPGr\n\
zqTDHkO30R8VeRM/Kz2f4nR05GIFiITl4bEjvpy7xqRDspJcCFIOcyXm8abVDhF+\n\
th6knSU0yLtNKuQVP6voMrnt9MV1X92LGZQLgdHZbPQz0Z5qIpaKhdyA8DEvWWvS\n\
Uwwc+yi1/gGaybwlzZwqXYoPOhwMebzKUk0xW14htcJrRrq+PXXQbRzTMynseCoP\n\
Ioke0dtCodbA3qQxQovE16q9zz4Otv2k4j63cz53J+mhkVWAeWxVGI0lltJmWtEY\n\
K6er8VqqWot3nqmWMXogrgRLggv/NbbooQIDAQAB\n\
-----END RSA PUBLIC KEY-----",
    "-----BEGIN RSA PUBLIC KEY-----\n\
MIIBCgKCAQEAvmpxVY7ld/8DAjz6F6q05shjg8/4p6047bn6/m8yPy1RBsvIyvuD\n\
uGnP/RzPEhzXQ9UJ5Ynmh2XJZgHoE9xbnfxL5BXHplJhMtADXKM9bWB11PU1Eioc\n\
3+AXBB8QiNFBn2XI5UkO5hPhbb9mJpjA9Uhw8EdfqJP8QetVsI/xrCEbwEXe0xvi\n\
fRLJbY08/Gp66KpQvy7g8w7VB8wlgePexW3pT13Ap6vuC+mQuJPyiHvSxjEKHgqe\n\
Pji9NP3tJUFQjcECqcm0yV7/2d0t/pbCm+ZH1sadZspQCEPPrtbkQBlvHb4OLiIW\n\
PGHKSMeRFvp3IWcmdJqXahxLCUS1Eh6MAQIDAQAB\n\
-----END RSA PUBLIC KEY-----",
];

static RSA_KEYS: LazyLock<Vec<RsaKey>> = LazyLock::new(|| {
    PEM_KEYS
        .iter()
        .filter_map(|pem| parse_pem_rsa(pem))
        .collect()
});

fn parse_pem_rsa(pem: &str) -> Option<RsaKey> {
    let body: String = pem
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect::<Vec<_>>()
        .join("");
    let der =
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, body.as_bytes()).ok()?;
    let (n_bytes, e_bytes) = parse_der_rsa_public(&der)?;
    let n = BigUint::from_bytes_be(&n_bytes);
    let e = BigUint::from_bytes_be(&e_bytes);
    let fingerprint = compute_fingerprint(&n_bytes, &e_bytes);
    Some(RsaKey { fingerprint, n, e })
}

fn parse_der_rsa_public(der: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    // SEQUENCE { INTEGER n, INTEGER e }
    let mut cur = der;
    cur = expect_tag(cur, 0x30)?;
    let (_seq_len, mut body) = read_length(cur)?;
    let n = read_integer(&mut body)?;
    let e = read_integer(&mut body)?;
    Some((strip_leading_zero(&n), strip_leading_zero(&e)))
}

fn expect_tag(data: &[u8], tag: u8) -> Option<&[u8]> {
    if data.is_empty() || data[0] != tag {
        return None;
    }
    Some(&data[1..])
}

fn read_length(data: &[u8]) -> Option<(usize, &[u8])> {
    if data.is_empty() {
        return None;
    }
    let first = data[0];
    if first & 0x80 == 0 {
        Some((first as usize, &data[1..]))
    } else {
        let n = (first & 0x7f) as usize;
        if data.len() < 1 + n {
            return None;
        }
        let mut len = 0usize;
        for i in 0..n {
            len = (len << 8) | data[1 + i] as usize;
        }
        Some((len, &data[1 + n..]))
    }
}

fn read_integer(data: &mut &[u8]) -> Option<Vec<u8>> {
    if data.is_empty() || data[0] != 0x02 {
        return None;
    }
    let after_tag = &data[1..];
    let (len, body) = read_length(after_tag)?;
    if body.len() < len {
        return None;
    }
    let int = body[..len].to_vec();
    *data = &body[len..];
    Some(int)
}

fn strip_leading_zero(bytes: &[u8]) -> Vec<u8> {
    if !bytes.is_empty() && bytes[0] == 0x00 {
        bytes[1..].to_vec()
    } else {
        bytes.to_vec()
    }
}

// telegram fingerprint = lower 8 bytes of sha1(serialize_bytes(n) + serialize_bytes(e)), little-endian i64
fn compute_fingerprint(n: &[u8], e: &[u8]) -> u64 {
    let mut buf = Vec::new();
    buf.extend(tl::serialize_bytes(n));
    buf.extend(tl::serialize_bytes(e));
    let mut hasher = Sha1::new();
    hasher.update(&buf);
    let hash = hasher.finalize();
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&hash[12..20]);
    u64::from_le_bytes(bytes)
}

fn find_key(fingerprints: &[u64]) -> Option<&'static RsaKey> {
    for fp in fingerprints {
        if let Some(k) = RSA_KEYS.iter().find(|k| k.fingerprint == *fp) {
            return Some(k);
        }
    }
    None
}

// telegram-style RSA: input padded as sha1(data) + data + random to 255 bytes; output is 256 bytes big-endian
fn rsa_encrypt(key: &RsaKey, data: &[u8]) -> Vec<u8> {
    let mut to_encrypt = Vec::with_capacity(255);
    let mut hasher = Sha1::new();
    hasher.update(data);
    to_encrypt.extend_from_slice(&hasher.finalize());
    to_encrypt.extend_from_slice(data);

    let pad_len = 255 - to_encrypt.len();
    let mut pad = vec![0u8; pad_len];
    rand::thread_rng().fill_bytes(&mut pad);
    to_encrypt.extend_from_slice(&pad);

    let payload = BigUint::from_bytes_be(&to_encrypt);
    let encrypted = payload.modpow(&key.e, &key.n);
    let mut out = encrypted.to_bytes_be();
    while out.len() < 256 {
        out.insert(0, 0);
    }
    out
}

// pollard's rho factorization for pq < 2^63
fn factorize(pq: u64) -> (u64, u64) {
    if pq % 2 == 0 {
        return (2, pq / 2);
    }
    use rand::Rng;
    let mut rng = rand::thread_rng();

    loop {
        let y_init: u64 = rng.gen_range(1..pq);
        let c: u64 = rng.gen_range(1..pq);
        let m: u64 = 128;
        let mut g: u64 = 1;
        let mut r: u64 = 1;
        let mut q: u64 = 1;
        let mut x = 0u64;
        let mut y = y_init;
        let mut ys = 0u64;

        while g == 1 {
            x = y;
            for _ in 0..r {
                y = (mulmod(y, y, pq) + c) % pq;
            }
            let mut k = 0u64;
            while k < r && g == 1 {
                ys = y;
                let upper = (m).min(r - k);
                for _ in 0..upper {
                    y = (mulmod(y, y, pq) + c) % pq;
                    let diff = if x > y { x - y } else { y - x };
                    q = mulmod(q, diff, pq);
                }
                g = gcd(q, pq);
                k += m;
            }
            r *= 2;
        }

        if g == pq {
            loop {
                ys = (mulmod(ys, ys, pq) + c) % pq;
                let diff = if x > ys { x - ys } else { ys - x };
                g = gcd(diff, pq);
                if g > 1 {
                    break;
                }
            }
        }

        if g != pq && g != 0 {
            let p = g.min(pq / g);
            let q_ = g.max(pq / g);
            return (p, q_);
        }
    }
}

fn mulmod(a: u64, b: u64, n: u64) -> u64 {
    ((a as u128 * b as u128) % n as u128) as u64
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

// unauthorized message wire format: auth_key_id(8 = 0) + msg_id(8) + msg_len(4) + body
fn build_unencrypted(body: &[u8]) -> Vec<u8> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let secs = now.as_secs();
    let nanos = now.subsec_nanos() as u64;
    let msg_id = (secs << 32) | ((nanos / 1000) << 2) | 4;

    let mut buf = Vec::with_capacity(20 + body.len());
    buf.write_u64::<LittleEndian>(0).unwrap();
    buf.write_u64::<LittleEndian>(msg_id).unwrap();
    buf.write_u32::<LittleEndian>(body.len() as u32).unwrap();
    buf.extend_from_slice(body);
    buf
}

fn parse_unencrypted(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() < 20 {
        return Err("unencrypted frame too short".into());
    }
    let auth_key_id = u64::from_le_bytes(data[0..8].try_into().unwrap());
    if auth_key_id != 0 {
        return Err(format!("expected auth_key_id=0, got {:#x}", auth_key_id));
    }
    let msg_len = u32::from_le_bytes(data[16..20].try_into().unwrap()) as usize;
    if data.len() < 20 + msg_len {
        return Err("unencrypted body length mismatch".into());
    }
    Ok(data[20..20 + msg_len].to_vec())
}

pub async fn send_unencrypted(transport: &mut MtpTransport, body: &[u8]) -> Result<(), String> {
    let frame = build_unencrypted(body);
    transport.send(&frame).await
}

pub async fn recv_unencrypted(transport: &mut MtpTransport) -> Result<Vec<u8>, String> {
    let frame = transport.recv().await?;
    parse_unencrypted(&frame)
}

// constructors used during DH exchange
const REQ_PQ_MULTI: u32 = 0xbe7e8ef1;
const RES_PQ: u32 = 0x05162463;
const P_Q_INNER_DATA: u32 = 0x83c95aec;
const REQ_DH_PARAMS: u32 = 0xd712e4be;
const SERVER_DH_PARAMS_OK: u32 = 0xd0e8075c;
const SERVER_DH_PARAMS_FAIL: u32 = 0x79cb045d;
const SERVER_DH_INNER_DATA: u32 = 0xb5890dba;
const CLIENT_DH_INNER_DATA: u32 = 0x6643b654;
const SET_CLIENT_DH_PARAMS: u32 = 0xf5045f1f;
const DH_GEN_OK: u32 = 0x3bcbf734;
const DH_GEN_RETRY: u32 = 0x46dc1fb9;
const DH_GEN_FAIL: u32 = 0xa69dae02;

pub struct DhResult {
    pub auth_key: [u8; 256],
    pub server_salt: u64,
}

pub async fn perform_dh(transport: &mut MtpTransport) -> Result<DhResult, String> {
    dbg_log!("auth::perform_dh start");

    let mut nonce = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut nonce);

    // step 1: req_pq_multi
    let mut req = Vec::new();
    req.write_u32::<LittleEndian>(REQ_PQ_MULTI).unwrap();
    req.extend_from_slice(&nonce);

    send_unencrypted(transport, &req).await?;
    let resp = recv_unencrypted(transport).await?;
    let mut cur = Cursor::new(&resp[..]);

    let ctor = cur
        .read_u32::<LittleEndian>()
        .map_err(|e| format!("read ctor: {e}"))?;
    if ctor != RES_PQ {
        return Err(format!("expected resPQ, got {:#x}", ctor));
    }

    let mut srv_nonce_buf = [0u8; 16];
    let mut resp_nonce = [0u8; 16];
    cur.read_exact(&mut resp_nonce)
        .map_err(|e| format!("read nonce: {e}"))?;
    if resp_nonce != nonce {
        return Err("nonce mismatch in resPQ".into());
    }
    cur.read_exact(&mut srv_nonce_buf)
        .map_err(|e| format!("read server_nonce: {e}"))?;

    let pq_bytes = tl::deserialize_bytes(&mut cur)?;
    if pq_bytes.len() > 8 {
        return Err("pq too large".into());
    }
    let mut pq_arr = [0u8; 8];
    pq_arr[8 - pq_bytes.len()..].copy_from_slice(&pq_bytes);
    let pq = u64::from_be_bytes(pq_arr);

    // fingerprints vector
    let vec_ctor = cur
        .read_u32::<LittleEndian>()
        .map_err(|e| format!("read fp vec ctor: {e}"))?;
    if vec_ctor != tl::VECTOR {
        return Err("fingerprints not a vector".into());
    }
    let fp_count = cur
        .read_u32::<LittleEndian>()
        .map_err(|e| format!("read fp count: {e}"))?;
    let mut fingerprints = Vec::with_capacity(fp_count as usize);
    for _ in 0..fp_count {
        fingerprints.push(
            cur.read_u64::<LittleEndian>()
                .map_err(|e| format!("read fp: {e}"))?,
        );
    }

    let key =
        find_key(&fingerprints).ok_or_else(|| "no matching RSA fingerprint found".to_string())?;
    dbg_log!(
        "auth::perform_dh got pq={} fingerprint={:#x}",
        pq,
        key.fingerprint
    );

    // step 2: factorize
    let (p, q) = factorize(pq);
    dbg_log!("auth::perform_dh factorized {} = {} * {}", pq, p, q);

    let p_bytes = trim_leading_zeros(&p.to_be_bytes());
    let q_bytes = trim_leading_zeros(&q.to_be_bytes());

    let mut new_nonce = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut new_nonce);

    // step 3: build p_q_inner_data
    let mut inner = Vec::new();
    inner.write_u32::<LittleEndian>(P_Q_INNER_DATA).unwrap();
    inner.extend(tl::serialize_bytes(&pq_bytes));
    inner.extend(tl::serialize_bytes(&p_bytes));
    inner.extend(tl::serialize_bytes(&q_bytes));
    inner.extend_from_slice(&nonce);
    inner.extend_from_slice(&srv_nonce_buf);
    inner.extend_from_slice(&new_nonce);

    let encrypted_data = rsa_encrypt(key, &inner);

    // step 4: req_DH_params
    let mut req2 = Vec::new();
    req2.write_u32::<LittleEndian>(REQ_DH_PARAMS).unwrap();
    req2.extend_from_slice(&nonce);
    req2.extend_from_slice(&srv_nonce_buf);
    req2.extend(tl::serialize_bytes(&p_bytes));
    req2.extend(tl::serialize_bytes(&q_bytes));
    req2.write_u64::<LittleEndian>(key.fingerprint).unwrap();
    req2.extend(tl::serialize_bytes(&encrypted_data));

    send_unencrypted(transport, &req2).await?;
    let resp2 = recv_unencrypted(transport).await?;
    let mut cur2 = Cursor::new(&resp2[..]);

    let ctor2 = cur2
        .read_u32::<LittleEndian>()
        .map_err(|e| format!("read DH ctor: {e}"))?;
    if ctor2 == SERVER_DH_PARAMS_FAIL {
        return Err("server_DH_params_fail".into());
    }
    if ctor2 != SERVER_DH_PARAMS_OK {
        return Err(format!("expected server_DH_params_ok, got {:#x}", ctor2));
    }

    let mut n2 = [0u8; 16];
    let mut sn2 = [0u8; 16];
    cur2.read_exact(&mut n2)
        .map_err(|e| format!("read DH nonce: {e}"))?;
    cur2.read_exact(&mut sn2)
        .map_err(|e| format!("read DH server_nonce: {e}"))?;
    if n2 != nonce || sn2 != srv_nonce_buf {
        return Err("nonce mismatch in server_DH_params_ok".into());
    }

    let encrypted_answer = tl::deserialize_bytes(&mut cur2)?;

    // step 5: derive tmp_aes_key/iv from new_nonce + server_nonce
    let (tmp_key, tmp_iv) = derive_tmp_aes(&new_nonce, &srv_nonce_buf);

    let answer_with_hash = ige_decrypt(&encrypted_answer, &tmp_key, &tmp_iv);
    if answer_with_hash.len() < 20 {
        return Err("DH answer too short".into());
    }
    let answer = &answer_with_hash[20..];
    let answer_hash = Sha1::digest(answer);
    if answer_with_hash[..20] != answer_hash[..] {
        return Err("server_DH_inner_data hash mismatch".into());
    }

    let mut cur3 = Cursor::new(answer);
    let inner_ctor = cur3
        .read_u32::<LittleEndian>()
        .map_err(|e| format!("read inner ctor: {e}"))?;
    if inner_ctor != SERVER_DH_INNER_DATA {
        return Err(format!(
            "expected server_DH_inner_data, got {:#x}",
            inner_ctor
        ));
    }

    let mut n3 = [0u8; 16];
    let mut sn3 = [0u8; 16];
    cur3.read_exact(&mut n3)
        .map_err(|e| format!("read DH inner nonce: {e}"))?;
    cur3.read_exact(&mut sn3)
        .map_err(|e| format!("read DH inner server_nonce: {e}"))?;
    if n3 != nonce || sn3 != srv_nonce_buf {
        return Err("nonce mismatch in server_DH_inner_data".into());
    }

    let g_int = cur3
        .read_i32::<LittleEndian>()
        .map_err(|e| format!("read g: {e}"))?;
    let dh_prime_bytes = tl::deserialize_bytes(&mut cur3)?;
    let g_a_bytes = tl::deserialize_bytes(&mut cur3)?;
    let server_time = cur3
        .read_i32::<LittleEndian>()
        .map_err(|e| format!("read server_time: {e}"))?;

    let dh_prime = BigUint::from_bytes_be(&dh_prime_bytes);
    let g_a = BigUint::from_bytes_be(&g_a_bytes);
    let g = BigUint::from(g_int as u32);

    // safety checks: g, g_a in (1, dh_prime - 1)
    let one = BigUint::from(1u32);
    let upper_bound = &dh_prime - &one;
    if !(g > one && g < upper_bound) {
        return Err("g is out of safe range".into());
    }
    if !(g_a > BigUint::from(1u32) && g_a < upper_bound) {
        return Err("g_a is out of safe range".into());
    }
    let _ = server_time; // accepted; we don't enforce time offset here

    // step 6: pick random b (2048 bit), compute g_b = g^b mod p, auth_key = g_a^b mod p
    let mut b_bytes = [0u8; 256];
    rand::thread_rng().fill_bytes(&mut b_bytes);
    let b = BigUint::from_bytes_be(&b_bytes);

    let g_b = g.modpow(&b, &dh_prime);
    let auth_key_int = g_a.modpow(&b, &dh_prime);

    if !(g_b > BigUint::from(1u32) && g_b < upper_bound) {
        return Err("g_b is out of safe range".into());
    }

    let mut auth_key_bytes = auth_key_int.to_bytes_be();
    while auth_key_bytes.len() < 256 {
        auth_key_bytes.insert(0, 0);
    }
    if auth_key_bytes.len() > 256 {
        auth_key_bytes = auth_key_bytes[auth_key_bytes.len() - 256..].to_vec();
    }
    let mut auth_key = [0u8; 256];
    auth_key.copy_from_slice(&auth_key_bytes);

    let g_b_bytes = g_b.to_bytes_be();

    // step 7: build client_DH_inner_data, encrypt with tmp_aes
    let mut client_inner = Vec::new();
    client_inner
        .write_u32::<LittleEndian>(CLIENT_DH_INNER_DATA)
        .unwrap();
    client_inner.extend_from_slice(&nonce);
    client_inner.extend_from_slice(&srv_nonce_buf);
    client_inner.write_u64::<LittleEndian>(0).unwrap(); // retry_id = 0
    client_inner.extend(tl::serialize_bytes(&g_b_bytes));

    let mut sha_inner = Sha1::new();
    sha_inner.update(&client_inner);
    let inner_hash = sha_inner.finalize();

    let mut to_encrypt = Vec::with_capacity(20 + client_inner.len());
    to_encrypt.extend_from_slice(&inner_hash);
    to_encrypt.extend_from_slice(&client_inner);
    // pad to multiple of 16
    let pad_len = (16 - to_encrypt.len() % 16) % 16;
    let mut pad = vec![0u8; pad_len];
    rand::thread_rng().fill_bytes(&mut pad);
    to_encrypt.extend_from_slice(&pad);

    let encrypted = ige_encrypt(&to_encrypt, &tmp_key, &tmp_iv);

    // step 8: set_client_DH_params
    let mut req3 = Vec::new();
    req3.write_u32::<LittleEndian>(SET_CLIENT_DH_PARAMS)
        .unwrap();
    req3.extend_from_slice(&nonce);
    req3.extend_from_slice(&srv_nonce_buf);
    req3.extend(tl::serialize_bytes(&encrypted));

    send_unencrypted(transport, &req3).await?;
    let resp3 = recv_unencrypted(transport).await?;
    let mut cur4 = Cursor::new(&resp3[..]);

    let final_ctor = cur4
        .read_u32::<LittleEndian>()
        .map_err(|e| format!("read final ctor: {e}"))?;
    let mut final_nonce = [0u8; 16];
    let mut final_server_nonce = [0u8; 16];
    cur4.read_exact(&mut final_nonce)
        .map_err(|e| format!("read final nonce: {e}"))?;
    cur4.read_exact(&mut final_server_nonce)
        .map_err(|e| format!("read final server_nonce: {e}"))?;
    if final_nonce != nonce || final_server_nonce != srv_nonce_buf {
        return Err("nonce mismatch in dh_gen response".into());
    }

    let mut auth_key_hash = Sha1::new();
    auth_key_hash.update(&auth_key);
    let auth_key_aux_hash = auth_key_hash.finalize();
    let expected_new_nonce_hash = |number: u8| {
        let mut hash = Sha1::new();
        hash.update(new_nonce);
        hash.update([number]);
        hash.update(&auth_key_aux_hash[..8]);
        let digest = hash.finalize();
        let mut result = [0u8; 16];
        result.copy_from_slice(&digest[4..20]);
        result
    };

    let mut received_new_nonce_hash = [0u8; 16];
    cur4.read_exact(&mut received_new_nonce_hash)
        .map_err(|e| format!("read new_nonce_hash: {e}"))?;
    match final_ctor {
        DH_GEN_OK => {
            if received_new_nonce_hash != expected_new_nonce_hash(1) {
                return Err("new_nonce_hash1 mismatch in dh_gen_ok".into());
            }
            dbg_log!("auth::perform_dh DH_GEN_OK server_time={}", server_time);
        }
        DH_GEN_RETRY => {
            if received_new_nonce_hash != expected_new_nonce_hash(2) {
                return Err("new_nonce_hash2 mismatch in dh_gen_retry".into());
            }
            return Err("dh_gen_retry — handshake should be retried".into());
        }
        DH_GEN_FAIL => {
            if received_new_nonce_hash != expected_new_nonce_hash(3) {
                return Err("new_nonce_hash3 mismatch in dh_gen_fail".into());
            }
            return Err("dh_gen_fail".into());
        }
        _ => return Err(format!("unexpected final ctor {:#x}", final_ctor)),
    }

    // server_salt = first 8 bytes of new_nonce[0..8] XOR server_nonce[0..8]
    let mut salt_bytes = [0u8; 8];
    for i in 0..8 {
        salt_bytes[i] = new_nonce[i] ^ srv_nonce_buf[i];
    }
    let server_salt = u64::from_le_bytes(salt_bytes);

    dbg_log!(
        "auth::perform_dh DONE auth_key_id={:#x} salt={:#x}",
        super::crypto::auth_key_id(&auth_key),
        server_salt
    );

    Ok(DhResult {
        auth_key,
        server_salt,
    })
}

fn derive_tmp_aes(new_nonce: &[u8; 32], server_nonce: &[u8; 16]) -> ([u8; 32], [u8; 32]) {
    // tmp_aes_key = SHA1(new_nonce + server_nonce) + SHA1(server_nonce + new_nonce)[0:12]
    // tmp_aes_iv = SHA1(server_nonce + new_nonce)[12:20] + SHA1(new_nonce + new_nonce) + new_nonce[0:4]
    let h1 = sha1_concat(&[new_nonce.as_ref(), server_nonce.as_ref()]);
    let h2 = sha1_concat(&[server_nonce.as_ref(), new_nonce.as_ref()]);
    let h3 = sha1_concat(&[new_nonce.as_ref(), new_nonce.as_ref()]);

    let mut key = [0u8; 32];
    key[0..20].copy_from_slice(&h1);
    key[20..32].copy_from_slice(&h2[0..12]);

    let mut iv = [0u8; 32];
    iv[0..8].copy_from_slice(&h2[12..20]);
    iv[8..28].copy_from_slice(&h3);
    iv[28..32].copy_from_slice(&new_nonce[0..4]);

    (key, iv)
}

fn sha1_concat(parts: &[&[u8]]) -> [u8; 20] {
    let mut hasher = Sha1::new();
    for p in parts {
        hasher.update(p);
    }
    let r = hasher.finalize();
    let mut out = [0u8; 20];
    out.copy_from_slice(&r);
    out
}

fn trim_leading_zeros(b: &[u8]) -> Vec<u8> {
    let mut i = 0;
    while i < b.len() - 1 && b[i] == 0 {
        i += 1;
    }
    b[i..].to_vec()
}

// === SRP for 2FA ===
// passwordKdfAlgoSHA256SHA256PBKDF2HMACSHA512iter100000SHA256ModPow

pub struct Srp {
    pub g: u32,
    pub p: Vec<u8>,
    pub salt1: Vec<u8>,
    pub salt2: Vec<u8>,
    pub srp_id: u64,
    pub srp_b: Vec<u8>,
}

pub struct SrpProof {
    pub a: Vec<u8>,
    pub m1: Vec<u8>,
}

pub fn compute_srp(srp: &Srp, password: &str) -> Result<SrpProof, String> {
    let p_bytes = &srp.p;
    if p_bytes.is_empty() {
        return Err("srp: p is empty".into());
    }
    let p = BigUint::from_bytes_be(p_bytes);
    if p == BigUint::from(0u32) {
        return Err("srp: p is zero".into());
    }
    if srp.g == 0 {
        return Err("srp: g is zero".into());
    }
    let g = BigUint::from(srp.g);
    if srp.srp_b.is_empty() {
        return Err("srp: srp_b is empty".into());
    }

    // g_for_hash = g zero-padded big-endian to 256 bytes
    let mut g_for_hash = vec![0u8; 256];
    let g_be = g.to_bytes_be();
    let off = 256 - g_be.len();
    g_for_hash[off..].copy_from_slice(&g_be);

    // x = compute_password_hash
    let x_bytes = password_hash(srp, password);
    let x = BigUint::from_bytes_be(&x_bytes);

    // v = g^x mod p
    let v = g.modpow(&x, &p);

    // k = SHA256(p || g_for_hash)
    let mut h = Sha256::new();
    sha2::Digest::update(&mut h, p_bytes);
    sha2::Digest::update(&mut h, &g_for_hash);
    let k_bytes = h.finalize();
    let k = BigUint::from_bytes_be(&k_bytes);

    let k_v = (k * v) % &p;

    // pick random a 2048-bit
    let mut a_bytes = vec![0u8; 256];
    rand::thread_rng().fill_bytes(&mut a_bytes);
    let a = BigUint::from_bytes_be(&a_bytes);

    let g_a = g.modpow(&a, &p);
    let g_a_bytes = pad_left(&g_a.to_bytes_be(), 256);

    let g_b = BigUint::from_bytes_be(&srp.srp_b);
    let g_b_bytes = pad_left(&g_b.to_bytes_be(), 256);

    let mut h = Sha256::new();
    sha2::Digest::update(&mut h, &g_a_bytes);
    sha2::Digest::update(&mut h, &g_b_bytes);
    let u_bytes = h.finalize();
    let u = BigUint::from_bytes_be(&u_bytes);

    let t = if g_b >= k_v {
        &g_b - &k_v
    } else {
        &p + &g_b - &k_v
    };

    let exponent = a + u * x;
    let s_a = t.modpow(&exponent, &p);
    let s_a_bytes = pad_left(&s_a.to_bytes_be(), 256);

    let mut h = Sha256::new();
    sha2::Digest::update(&mut h, &s_a_bytes);
    let k_a: [u8; 32] = h.finalize().into();

    let h_p: [u8; 32] = {
        let mut h = Sha256::new();
        sha2::Digest::update(&mut h, p_bytes);
        h.finalize().into()
    };
    let h_g: [u8; 32] = {
        let mut h = Sha256::new();
        sha2::Digest::update(&mut h, &g_for_hash);
        h.finalize().into()
    };
    let h_salt1: [u8; 32] = {
        let mut h = Sha256::new();
        sha2::Digest::update(&mut h, &srp.salt1);
        h.finalize().into()
    };
    let h_salt2: [u8; 32] = {
        let mut h = Sha256::new();
        sha2::Digest::update(&mut h, &srp.salt2);
        h.finalize().into()
    };

    let mut xored = [0u8; 32];
    for i in 0..32 {
        xored[i] = h_p[i] ^ h_g[i];
    }

    let mut h = Sha256::new();
    sha2::Digest::update(&mut h, &xored);
    sha2::Digest::update(&mut h, &h_salt1);
    sha2::Digest::update(&mut h, &h_salt2);
    sha2::Digest::update(&mut h, &g_a_bytes);
    sha2::Digest::update(&mut h, &g_b_bytes);
    sha2::Digest::update(&mut h, &k_a);
    let m1: [u8; 32] = h.finalize().into();

    Ok(SrpProof {
        a: g_a_bytes,
        m1: m1.to_vec(),
    })
}

fn password_hash(srp: &Srp, password: &str) -> [u8; 32] {
    // hash1 = sha256(salt1 || password || salt1)
    let mut h = Sha256::new();
    sha2::Digest::update(&mut h, &srp.salt1);
    sha2::Digest::update(&mut h, password.as_bytes());
    sha2::Digest::update(&mut h, &srp.salt1);
    let hash1: [u8; 32] = h.finalize().into();

    // hash2 = sha256(salt2 || hash1 || salt2)
    let mut h = Sha256::new();
    sha2::Digest::update(&mut h, &srp.salt2);
    sha2::Digest::update(&mut h, &hash1);
    sha2::Digest::update(&mut h, &srp.salt2);
    let hash2: [u8; 32] = h.finalize().into();

    // hash3 = pbkdf2_sha512(hash2, salt1, 100000), 64 bytes
    let mut hash3 = [0u8; 64];
    pbkdf2_sha512(&hash2, &srp.salt1, 100_000, &mut hash3);

    // result = sha256(salt2 || hash3 || salt2)
    let mut h = Sha256::new();
    sha2::Digest::update(&mut h, &srp.salt2);
    sha2::Digest::update(&mut h, &hash3);
    sha2::Digest::update(&mut h, &srp.salt2);
    let out: [u8; 32] = h.finalize().into();
    out
}

fn pbkdf2_sha512(password: &[u8], salt: &[u8], iterations: u32, output: &mut [u8]) {
    use hmac::{Hmac, Mac};
    use sha2::Sha512;
    type HmacSha512 = Hmac<Sha512>;

    let hlen = 64;
    let num_blocks = (output.len() + hlen - 1) / hlen;

    for block_num in 1..=num_blocks {
        let mut mac = <HmacSha512 as Mac>::new_from_slice(password).unwrap();
        mac.update(salt);
        mac.update(&(block_num as u32).to_be_bytes());
        let mut u_vec = mac.finalize().into_bytes().to_vec();
        let mut t = u_vec.clone();

        for _ in 1..iterations {
            let mut mac = <HmacSha512 as Mac>::new_from_slice(password).unwrap();
            mac.update(&u_vec);
            let new_u = mac.finalize().into_bytes();
            u_vec = new_u.to_vec();
            for i in 0..hlen {
                t[i] ^= u_vec[i];
            }
        }

        let start = (block_num - 1) * hlen;
        let end = (start + hlen).min(output.len());
        output[start..end].copy_from_slice(&t[..end - start]);
    }
}

fn pad_left(b: &[u8], n: usize) -> Vec<u8> {
    if b.len() >= n {
        return b[b.len() - n..].to_vec();
    }
    let mut out = vec![0u8; n];
    out[n - b.len()..].copy_from_slice(b);
    out
}

// compute the SRP verifier for setting a NEW 2FA password.
// telegram's account.updatePasswordSettings expects new_password_hash to be
// v = g^x mod p (256-byte big-endian), where x = PH2(password, salt1, salt2)
// derived from the server-provided new_algo params (g, p, salt1, salt2).
// salt1 must already include the 32 random bytes appended to new_algo.salt1.
pub fn compute_new_password_verifier(
    g: u32,
    p: &[u8],
    salt1: &[u8],
    salt2: &[u8],
    password: &str,
) -> Result<Vec<u8>, String> {
    if p.is_empty() {
        return Err("new 2FA: p is empty".into());
    }
    if g == 0 {
        return Err("new 2FA: g is zero".into());
    }

    // reuse the canonical PH2 derivation via a throwaway Srp holding the new_algo salts
    let srp = Srp {
        g,
        p: p.to_vec(),
        salt1: salt1.to_vec(),
        salt2: salt2.to_vec(),
        srp_id: 0,
        srp_b: Vec::new(),
    };
    let x_bytes = password_hash(&srp, password);
    let x = BigUint::from_bytes_be(&x_bytes);
    let p_big = BigUint::from_bytes_be(p);
    let g_big = BigUint::from(g);

    let v = g_big.modpow(&x, &p_big);
    Ok(pad_left(&v.to_bytes_be(), 256))
}
