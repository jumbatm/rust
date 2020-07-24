// check-fail
// Tests error conditions for specifying diagnostics using #[derive(AsSessionError)]

#![feature(rustc_private)]
#![crate_type = "lib"]

extern crate rustc_macros;
use rustc_macros::AsSessionError;

// The macro doesn't pull these crates in itself, because most internal use within the compiler is
// from contexts where referencing the crates is enough anyway.
extern crate rustc_errors;
extern crate rustc_session;

#[derive(AsSessionError)]
#[error = "Hello, world!"]
#[code = "E0123"]
struct Hello {}

#[derive(AsSessionError)]
#[error = "Hello, world!"]
#[code = "E0123"]
#[code = "E0456"] //~ ERROR Diagnostic ID multiply provided
struct ErrorSpecifiedTwice {}
