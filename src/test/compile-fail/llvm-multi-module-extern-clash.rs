mod foo {
    extern {
        pub fn func();
    }
}
mod bar {
    extern {
        pub fn func(x: i32); // ERROR symbol `func` has already been declared.
    }
}
fn main() {
    unsafe {
        foo::func();
        bar::func(100);
    }
}
