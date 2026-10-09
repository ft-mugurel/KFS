pub fn c_str_to_rust(c_str: *const u8) -> &'static str {
    if c_str.is_null() {
        return "";
    }

    let mut len = 0;
    while unsafe { *c_str.add(len) } != 0 {
        len += 1;
    }

    let slice = unsafe { core::slice::from_raw_parts(c_str, len) };
    core::str::from_utf8(slice).unwrap_or("")
}
