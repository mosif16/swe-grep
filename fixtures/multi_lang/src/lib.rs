pub fn login_user(username: &str, password: &str) -> Option<String> {
    if username == "admin" && password == "swe" {
        Some("token-admin".to_string())
    } else {
        None
    }
}

pub fn compute_checksum(input: &str) -> u64 {
    input.bytes().fold(0u64, |acc, byte| acc.wrapping_mul(31).wrapping_add(byte as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_user_allows_admin() {
        assert_eq!(login_user("admin", "swe"), Some("token-admin".to_string()));
    }
}
