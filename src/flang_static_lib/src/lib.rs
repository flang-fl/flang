use std::io::{Read, Write, stdin, stdout};

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

#[unsafe(no_mangle)]
pub extern "C" fn flang_print_ascii(value: i64) {
    let byte = value.rem_euclid(256) as u8;

    let mut output = stdout().lock();
    output.write_all(&[byte]).unwrap();
    output.flush().unwrap();
}