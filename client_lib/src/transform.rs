use serde::{Deserialize, Serialize};

/// The axis along which to perform a transformation.
#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub enum TransformDirection {
    Vertical,
    Horizontal,
}

/// Action and direction of a geometric transformation to perform.
#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub enum Transform {
    /// Flip the image in the specified direction.
    Flip(TransformDirection),
}
