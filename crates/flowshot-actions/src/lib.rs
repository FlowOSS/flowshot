#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! Action handlers for `FlowShot`.

pub mod actions;
pub mod error;

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
