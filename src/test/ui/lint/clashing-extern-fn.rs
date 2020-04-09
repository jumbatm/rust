// check-pass
// aux-build:external_extern_fn.rs
#![crate_type = "lib"]
#![warn(clashing_extern_decl)]

extern crate external_extern_fn;

extern {
    fn clash(x: u8);
    fn no_clash(x: u8);
}

fn redeclared_different_signature() {
    extern {
        fn clash(x: u64); //~ WARN `clash` redeclared with a different signature
    }

    unsafe {
        clash(123);
        no_clash(123);
    }
}

fn redeclared_same_signature() {
    extern {
        fn no_clash(x: u8);
    }
    unsafe {
        no_clash(123);
    }
}

extern {
    fn extern_fn(x: u64);
}

fn extern_clash() {
    extern {
        fn extern_fn(x: u32); //~ WARN `extern_fn` redeclared with a different signature
    }
    unsafe {
        extern_fn(123);
    }
}

fn extern_no_clash() {
    unsafe {
        external_extern_fn::extern_fn(123);
        crate::extern_fn(123);
    }
}
extern {
    fn some_other_new_name(x: i16);

    #[link_name = "extern_link_name"]
    fn some_new_name(x: i16);

    #[link_name = "link_name_same"]
    fn both_names_different(x: i16);
}

fn link_name_clash() {
    extern {
        fn extern_link_name(x: u32);
        //~^ WARN `extern_link_name` redeclared with a different signature

        #[link_name = "some_other_new_name"]
        //~^ WARN `some_other_extern_link_name` redeclares `some_other_new_name` with a different
        fn some_other_extern_link_name(x: u32);

        #[link_name = "link_name_same"]
        //~^ WARN `other_both_names_different` redeclares `link_name_same` with a different
        fn other_both_names_different(x: u32);
    }
}

mod a {
    extern {
        fn different_mod(x: u8);
    }
}
mod b {
    extern {
        fn different_mod(x: u64); //~ WARN `different_mod` redeclared with a different signature
    }
}

extern {
    fn variadic_decl(x: u8, ...);
}

fn variadic_clash() {
    extern {
        fn variadic_decl(x: u8); //~ WARN `variadic_decl` redeclared with a different signature
    }
}

#[no_mangle]
fn no_mangle_name(x: u8) { }

extern {
    #[link_name = "unique_link_name"]
    fn link_name_specified(x: u8);
}

fn tricky_no_clash() {
    extern {
        // Shouldn't warn, because the declaration above actually declares a different symbol (and
        // Rust's name resolution rules around shadowing will handle this gracefully).
        fn link_name_specified() -> u32;

        // The case of a no_mangle name colliding with an extern decl (see #28179) is related but
        // shouldn't be reported by ClashingExternDecl, because this is an example of unmangled
        // name clash causing bad behaviour in functions with a defined body.
        fn no_mangle_name() -> u32;
    }
}
