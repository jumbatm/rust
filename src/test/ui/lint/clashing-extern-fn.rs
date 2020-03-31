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
