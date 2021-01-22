use std::error::Error;
use argon2rs;
use getrandom;

pub struct Password {
    hash: Vec<u8>,
    salt: Vec<u8>,
}

impl Password {
    pub fn hash(password: &str) -> Result<Password, Box<dyn Error>> {
        let mut bytes = vec![0u8; 32];
        getrandom::getrandom(&mut bytes)?;

        let salt = String::from_utf8(bytes)?;
        let hash = argon2rs::argon2i_simple(password, &salt);

        return Ok(Password{
            hash: hash.to_vec(),
            salt: salt.into_bytes(),
        })
    }

    pub fn is_valid(self, password: &str) -> bool {
        let salt = String::from_utf8(self.salt).unwrap();
        let hash = argon2rs::argon2i_simple(password, &salt);

        return hash.to_vec() == self.hash
    }
}