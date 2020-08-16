//! Defines the helper traits use in [rustc_macros::session_diagnostic].
//! This is defined here rather than in the macros crate so we're able to reference the concrete
//! types we're working with without introducing a cyclic dependency.

use rustc_span::Span;
use rustc_errors::Applicability;

pub trait SpanAndApplicability {
    fn get_span(&self) -> Span;

    fn get_applicability(&self) -> Applicability {
        Applicability::Unspecified
    }
}

impl SpanAndApplicability for (Span, Applicability) {
    fn get_span(&self) -> Span {
        self.0
    }
    fn get_applicability(&self) -> Applicability {
        self.1
    }
}

impl SpanAndApplicability for (Applicability, Span) {
    fn get_span(&self) -> Span {
        self.1
    }
    fn get_applicability(&self) -> Applicability {
        self.0
    }
}

impl SpanAndApplicability for Span {
    fn get_span(&self) -> Span {
        *self
    }
}
