pub mod hardware;
pub mod synthetic_data;

/// Common trait for all validators
pub trait Validator {
    type Error;

    /// Returns the name of the validator
    fn name(&self) -> &str;
}
