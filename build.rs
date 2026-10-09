use std::env;
use std::fs;
use std::path::PathBuf;

use rustc_demangle::try_demangle;

fn main() {
    println!("cargo:rerun-if-changed=build/kallsyms.map");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));
    let out_file = out_dir.join("kallsyms_generated.rs");

    let map_path = PathBuf::from("build/kallsyms.map");
    let generated = match fs::read_to_string(&map_path) {
        Ok(content) => generate_outputs(&content),
        Err(_) => "pub static KALLSYMS: &[crate::dump::kallsyms::Symbol] = &[];\n".to_string(),
    };

    fs::write(out_file, generated).expect("failed to write kallsyms_generated.rs");
}

fn generate_outputs(map: &str) -> String {
    let mut entries = parse_entries(map);
    entries.sort_by_key(|(addr, _)| *addr);
    dedup_by_addr(&mut entries);

    let pretty = to_map_text(&entries);
    let _ = fs::write("build/kallsyms.pretty.map", pretty);

    let slim: Vec<(u32, String)> = entries
        .into_iter()
        .filter(|(_, name)| keep_runtime_symbol(name))
        .collect();
    let _ = fs::write("build/kallsyms.slim.map", to_map_text(&slim));

    let mut out = String::new();
    out.push_str("pub static KALLSYMS: &[crate::dump::kallsyms::Symbol] = &[\n");

    for (addr, name) in slim {
        out.push_str("    (");
        out.push_str(&format!("0x{addr:08x}"));
        out.push_str(", ");
        out.push_str(&format!("\"{}\"", escape(&name)));
        out.push_str("),\n");
    }

    out.push_str("];\n");
    out
}

fn parse_entries(map: &str) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    for line in map.lines() {
        let mut parts = line.split_whitespace();
        let Some(addr_hex) = parts.next() else {
            continue;
        };
        let Some(raw_name) = parts.next() else {
            continue;
        };

        if let Ok(addr) = u32::from_str_radix(addr_hex, 16) {
            let name = humanize_symbol(raw_name);
            out.push((addr, name));
        }
    }
    out
}

fn humanize_symbol(raw: &str) -> String {
    let demangled = try_demangle(raw)
        .map(|d| d.to_string())
        .unwrap_or_else(|_| raw.to_string());
    let mut s = strip_rust_hash(&demangled).to_string();
    s = s.replace("::{{closure}}", "::closure");
    s = s.replace("$LT$", "<");
    s = s.replace("$GT$", ">");
    s
}

fn strip_rust_hash(name: &str) -> &str {
    if let Some(idx) = name.rfind("::h") {
        let hash = &name[idx + 3..];
        if hash.len() == 16 && hash.chars().all(|c| c.is_ascii_hexdigit()) {
            return &name[..idx];
        }
    }
    name
}

fn dedup_by_addr(entries: &mut Vec<(u32, String)>) {
    if entries.is_empty() {
        return;
    }

    let mut deduped: Vec<(u32, String)> = Vec::with_capacity(entries.len());
    for (addr, name) in entries.drain(..) {
        if let Some((last_addr, last_name)) = deduped.last_mut() {
            if *last_addr == addr {
                if symbol_score(&name) > symbol_score(last_name) {
                    *last_name = name;
                }
                continue;
            }
        }
        deduped.push((addr, name));
    }

    *entries = deduped;
}

fn symbol_score(name: &str) -> usize {
    let mut score = 0usize;
    if name.starts_with("kernel::") {
        score += 100;
    }
    if name == "_start" || name == "kmain" {
        score += 90;
    }
    if name.starts_with("isr_") || name.contains("interrupt") {
        score += 80;
    }
    if name.starts_with("core::fmt::") {
        score = score.saturating_sub(20);
    }
    score + name.len().min(32)
}

fn keep_runtime_symbol(name: &str) -> bool {
    name.starts_with("kernel::")
        || name == "_start"
        || name == "kmain"
        || name == "rust_begin_unwind"
        || name.starts_with("isr_")
        || name.ends_with("_interrupt_handler")
        || name.contains("exception_common_handler")
        || name.starts_with("core::panicking::")
        || name.starts_with("core::option::unwrap_failed")
        || name.starts_with("core::result::unwrap_failed")
}

fn to_map_text(entries: &[(u32, String)]) -> String {
    let mut out = String::new();
    for (addr, name) in entries {
        out.push_str(&format!("{addr:08x} {name}\n"));
    }
    out
}

fn escape(s: &str) -> String {
    let mut escaped = String::new();
    for ch in s.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            _ => escaped.push(ch),
        }
    }
    escaped
}
