//! Consider you have this generic function `foo`:
//!
//! ```rust,ignore(pseudocode)
//! fn foo<X, Y, Z>(x: X, y: Y, z: Z) {
//!    let x: String = something_with_x(x);
//!    let y: String = something_with_y(y);
//!    let z: String = something_with_z(z);
//!    // ... lots more code ...
//! }
//! ```
//!
//! If `foo` is monomorphised as-is, you'd have to copy the entire function body for every single
//! set of concrete use of `foo`.
//!
//! However, we can observe that `foo` is, only generic up until `x`, `y` and `z` are all moved.
//! After that point, the rest of the function body could really just be replaced by a non-generic
//! function `foo_impl`:
//!
//! ```rust,ignore(pseudocode)
//! fn foo<X, Y, Z>(x: X, y: Y, z: Z) {
//!    let x: String = something_with_x(x);
//!    let y: String = something_with_y(y);
//!    let z: String = something_with_z(z);
//!    foo_impl(x, y, z)
//! }
//!
//! fn foo_impl(x: String, y: String, z: String) {
//!    // ... lots more code ...
//! }
//! ```
//!
//! Now, when `foo` gets monomorphised, we only have to monomorphise the start of `foo`, and they
//! can all call the same `foo_impl`.
//!
//! That's what this transform achieves: It first detects where (if at all) there's a "pinch point"
//! where the function becomes non-generic. It then splits the function at that point, putting
//! everything after the pinch point into a different Body and replacing it with a call to an impl
//! function.
