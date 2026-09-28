use pbkdf2::pbkdf2_hmac;
use sha2::Sha256;

/// Derives the 32-byte key used by encrypted registrar settings.
pub fn derive_key(password: &[u8], salt: &[u8], rounds: u32) -> [u8; 32] {
    let mut key = [0_u8; 32];
    pbkdf2_hmac::<Sha256>(password, salt, rounds, &mut key);
    key
}
