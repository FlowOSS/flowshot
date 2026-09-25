#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! Screen capture functionality for `FlowShot`.

pub mod capture;
pub mod error;

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
