use core::str;
use crate::dump::{
    self, DumpStackOptions, DEFAULT_DUMP_WORDS, DEFAULT_TRACE_FRAMES, MAX_DUMP_WORDS,
    MEMDUMP_DEFAULT_LEN, MEMDUMP_MAX_LEN,
};
use crate::shell::init::{print, print_fmt};
use super::parse::{parse_u32, parse_usize};

#[inline(always)]
pub(crate) fn command_memdump(mut parts: str::SplitWhitespace<'_>) {
    let Some(addr_str) = parts.next() else {
        print("usage: memdump <addr> [len<=512]\n");
        return;
    };

    let Some(addr) = parse_u32(addr_str) else {
        print("invalid address\n");
        return;
    };

    let len = if let Some(len_str) = parts.next() {
        let Some(parsed) = parse_usize(len_str) else {
            print("invalid length\n");
            return;
        };
        parsed
    } else {
        MEMDUMP_DEFAULT_LEN
    };

    if len == 0 || len > MEMDUMP_MAX_LEN {
        print("length must be in range 1..=512\n");
        return;
    }

    dump::dump_virtual_memory(addr, len, |args| print_fmt(args));
}

#[inline(always)]
pub(crate) fn command_pte(mut parts: str::SplitWhitespace<'_>) {
    let Some(addr_str) = parts.next() else {
        print("usage: pte <addr>\n");
        return;
    };

    let Some(addr) = parse_u32(addr_str) else {
        print("invalid address\n");
        return;
    };

    dump::debug_page_entry(addr, |args| print_fmt(args));
}

#[inline(always)]
pub(crate) fn command_stack(mut parts: str::SplitWhitespace<'_>) {
    let words = if let Some(words_str) = parts.next() {
        let Some(parsed) = parse_usize(words_str) else {
            print("invalid word count\n");
            return;
        };
        parsed
    } else {
        DEFAULT_DUMP_WORDS
    };

    if words == 0 || words > MAX_DUMP_WORDS {
        print("word count must be in range 1..=64\n");
        return;
    }

    let options = DumpStackOptions {
        words,
        frames: DEFAULT_TRACE_FRAMES,
        print_stack_values: true,
        walk_frames: true,
        scan_stack: true,
    };

    dump::dump_stack_with_options(options, |args| {
        print_fmt(&args);
    });
}
