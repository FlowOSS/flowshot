#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! User interface components for `FlowShot`.

pub mod error;
pub mod ui;

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
