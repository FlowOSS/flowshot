//! Wayland-specific screen capture functionality for `FlowShot`.

#![forbid(unsafe_code)]

pub mod capture;
pub mod error;

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
