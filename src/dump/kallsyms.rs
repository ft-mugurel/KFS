pub type Symbol = (u32, &'static str);

include!(concat!(env!("OUT_DIR"), "/kallsyms_generated.rs"));

pub fn lookup(addr: u32) -> Option<(&'static str, u32)> {
    if KALLSYMS.is_empty() {
        return None;
    }

    let mut low = 0usize;
    let mut high = KALLSYMS.len();

    while low < high {
        let mid = low + (high - low) / 2;
        let sym_addr = KALLSYMS[mid].0;
        if sym_addr <= addr {
            low = mid + 1;
        } else {
            high = mid;
        }
    }

    if low == 0 {
        return None;
    }

    let (sym_addr, sym_name) = KALLSYMS[low - 1];
    Some((sym_name, addr.wrapping_sub(sym_addr)))
}
