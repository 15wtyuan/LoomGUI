pub mod layout;
pub mod parse;
pub mod render;
pub mod scene;
pub mod style;
pub mod text;

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
