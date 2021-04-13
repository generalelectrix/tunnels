use derive_more::{Add, AddAssign, Display, From, Mul, MulAssign, RemAssign, Sub};
use serde::{Deserialize, Serialize};

/// A float type constrained to the range [0.0, 1.0].
#[derive(
    Display,
    Debug,
    Copy,
    Clone,
    PartialEq,
    PartialOrd,
    Serialize,
    Deserialize,
    Add,
    Sub,
    Mul,
    From,
    RemAssign,
    AddAssign,
    MulAssign,
)]
pub struct UnipolarFloat(pub f64);

// A float type constrained to the range [-1.0, 1.0].
#[derive(
    Display,
    Debug,
    Copy,
    Clone,
    PartialEq,
    PartialOrd,
    Serialize,
    Deserialize,
    Add,
    Sub,
    Mul,
    From,
    RemAssign,
    AddAssign,
)]
pub struct BipolarFloat(pub f64);
