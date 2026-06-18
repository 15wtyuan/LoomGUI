pub mod parse;

pub fn version() -> &'static str {
    "v0-skeleton"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_set() {
        assert_eq!(version(), "v0-skeleton");
    }
}
