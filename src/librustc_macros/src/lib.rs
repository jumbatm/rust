#![feature(proc_macro_diagnostic)]
#![allow(rustc::default_hash_types)]
#![recursion_limit = "128"]

use synstructure::decl_derive;

use proc_macro::TokenStream;

mod session_diagnostic;
mod hash_stable;
mod lift;
mod query;
mod serialize;
mod symbols;
mod type_foldable;

#[proc_macro]
pub fn rustc_queries(input: TokenStream) -> TokenStream {
    query::rustc_queries(input)
}

#[proc_macro]
pub fn symbols(input: TokenStream) -> TokenStream {
    symbols::symbols(input)
}

decl_derive!([HashStable, attributes(stable_hasher)] => hash_stable::hash_stable_derive);
decl_derive!(
    [HashStable_Generic, attributes(stable_hasher)] =>
    hash_stable::hash_stable_generic_derive
);

decl_derive!([Decodable] => serialize::decodable_derive);
decl_derive!([Encodable] => serialize::encodable_derive);
decl_derive!([TyDecodable] => serialize::type_decodable_derive);
decl_derive!([TyEncodable] => serialize::type_encodable_derive);
decl_derive!([MetadataDecodable] => serialize::meta_decodable_derive);
decl_derive!([MetadataEncodable] => serialize::meta_encodable_derive);
decl_derive!([TypeFoldable, attributes(type_foldable)] => type_foldable::type_foldable_derive);
decl_derive!([Lift, attributes(lift)] => lift::lift_derive);
decl_derive!([SessionDiagnostic, attributes(code, error, label, suggestion)] => session_diagnostic::session_diagnostic_derive);
