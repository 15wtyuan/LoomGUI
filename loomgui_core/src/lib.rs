pub mod asset;
pub mod dump;
pub mod hit;
pub mod input;
pub mod layout;
#[cfg(feature = "parse")]
pub mod parse;
pub mod render;
pub mod scene;
pub mod scroll;
pub mod stage;
pub mod style;
pub mod text;
pub mod transform;
pub mod tween;

pub use stage::Stage;

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
