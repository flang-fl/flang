use std::io::{stdin, Read};

#[unsafe(no_mangle)]
pub extern "C" fn flang_read_byte() -> i64 {
    stdin().bytes().next().unwrap().unwrap() as i64
}

#[unsafe(no_mangle)]
pub extern "C" fn flang_print_i64(value: i64) {
    println!("i64: {value}");
}

#[unsafe(no_mangle)]
pub extern "C" fn flang_print_bool(value: bool) {
    println!("bool: {value}");
}