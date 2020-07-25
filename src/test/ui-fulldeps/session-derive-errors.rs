// check-fail
// Tests error conditions for specifying diagnostics using #[derive(AsSessionError)]

#![feature(rustc_private)]
#![crate_type = "lib"]

extern crate rustc_span;
use rustc_span::Span;

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

// FIXME: Uncomment when emitting lints is supported.
/*
#[derive(AsSessionError)]
#[error = "Hello, world!"]
#[lint = "clashing_extern_declarations"]
#[lint = "improper_ctypes"] // FIXME: ERROR Diagnostic ID multiply provided
struct LintSpecifiedTwice {}
*/

#[derive(AsSessionError)]
#[code = "E0123"]
struct ErrorWithField {
    name: String,
    #[error = "This error has a field, and references {name}"]
    span: Span
}

#[derive(AsSessionError)]
#[code = "E0123"]
struct ErrorWithNonexistentField {
    #[error = "This error has a field, and references {name}"]
    //~^ ERROR no field `name` on this type
    span: Span
}

#[derive(AsSessionError)]
#[code = "E0123"]
#[error = "Something something"]
struct LabelOnSpan {
    #[label = "See here"]
    sp: Span
}

#[derive(AsSessionError)]
#[code = "E0123"]
#[error = "Something something"]
struct LabelOnNonSpan {
    #[label = "See here"]
    //~^ ERROR The `#[label = ...]` attribute can only be applied to fields of type Span
    id: u32,
}
