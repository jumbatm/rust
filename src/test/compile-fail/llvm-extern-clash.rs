#[no_mangle]
fn foo(_: i32) {}

fn main() {
    extern {
        fn foo(); //~ ERROR symbol `foo` defined multiple times.
    }
    unsafe { foo(); }
}
