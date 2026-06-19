use core::{fmt, ptr};

use crate::debug::kallsyms;
use crate::paging::page_table;
use crate::x86;

pub const DEFAULT_DUMP_WORDS: usize = 10;
pub const MAX_DUMP_WORDS: usize = 64;
pub const DEFAULT_TRACE_FRAMES: usize = 10;
pub const MAX_TRACE_FRAMES: usize = 32;
const MAX_SYMBOL_OFFSET: u32 = 0x2000;
const DEFAULT_SCAN_WORDS: usize = 128;
const MAX_SCAN_WORDS: usize = 256;

#[derive(Clone, Copy)]
pub struct DumpStackOptions {
    pub words: usize,
    pub trace_frames: usize,
}

impl Default for DumpStackOptions {
    fn default() -> Self {
        Self {
            words: DEFAULT_DUMP_WORDS,
            trace_frames: DEFAULT_TRACE_FRAMES,
        }
    }
}

#[inline(never)]
pub fn dump_stack(mut emit: impl FnMut(fmt::Arguments<'_>)) {
    dump_stack_with_options(DumpStackOptions::default(), &mut emit);
}

#[inline(never)]
pub fn dump_stack_with_options(
    options: DumpStackOptions,
    mut emit: impl FnMut(fmt::Arguments<'_>),
) {
    // let words = options.words.clamp(1, MAX_DUMP_WORDS);
    let trace_frames = options.trace_frames.clamp(1, MAX_TRACE_FRAMES);

    // let esp = x86::read_esp();
    let ebp = x86::read_ebp();

    // emit(format_args!("------------[ cut here ]------------\n"));
    // emit(format_args!("Kernel stack dump\n"));
    // emit(format_args!("ESP: {:#010x} EBP: {:#010x}\n", esp, ebp));
    // emit(format_args!("Stack:\n"));

    // for i in 0..words {
    //     let offset = match (i as u32).checked_mul(4) {
    //         Some(v) => v,
    //         None => break,
    //     };

    //     let addr = match esp.checked_add(offset) {
    //         Some(v) => v,
    //         None => {
    //             emit(format_args!("  <address overflow>\n"));
    //             break;
    //         }
    //     };

    //     let marker = if addr == ebp { " <ebp>" } else { "" };
    //     if !is_mapped_u32(addr) {
    //         emit(format_args!("  {:#010x}: ????????{}\n", addr, marker));
    //         continue;
    //     }

    //     let value = unsafe { ptr::read_volatile(addr as *const u32) };
    //     emit(format_args!(
    //         "  {:#010x}: {:#010x}{}\n",
    //         addr, value, marker
    //     ));
    // }

    emit(format_args!("Call Trace (frame walk):\n"));
    let mut frame = ebp;
    let mut walked = 0usize;
    for depth in 0..trace_frames {
        if frame == 0 {
            emit(format_args!("  <end of frames>\n"));
            break;
        }

        if !is_mapped_u32(frame) || !is_mapped_u32(frame.wrapping_add(4)) {
            emit(format_args!(
                "  [{:02}] <unmapped frame {:#010x}>\n",
                depth, frame
            ));
            break;
        }

        let next = unsafe { ptr::read_volatile(frame as *const u32) };
        let ret = unsafe { ptr::read_volatile((frame.wrapping_add(4)) as *const u32) };
        emit_trace_entry(&mut emit, depth, ret);
        walked += 1;

        if next <= frame {
            emit(format_args!("  <end of frame chain>\n"));
            break;
        }

        if next.wrapping_sub(frame) > 0x10000 {
            emit(format_args!(
                "  <corrupt frame chain: next={:#010x}>\n",
                next
            ));
            break;
        }

        frame = next;
    }
/* 
    emit(format_args!("Call Trace (stack scan):\n"));
    let mut emitted = 0usize;
    let mut last_name: Option<&str> = None;
    let scan_words = (words.saturating_mul(8))
        .max(DEFAULT_SCAN_WORDS)
        .min(MAX_SCAN_WORDS);

    for i in 0..scan_words {
        let offset = match (i as u32).checked_mul(4) {
            Some(v) => v,
            None => break,
        };
        let slot = match esp.checked_add(offset) {
            Some(v) => v,
            None => break,
        };

        if !is_mapped_u32(slot) {
            continue;
        }

        let candidate = unsafe { ptr::read_volatile(slot as *const u32) };
        if candidate < 2 {
            continue;
        }

        let Some((name, sym_off)) = kallsyms::lookup(candidate - 1) else {
            continue;
        };

        if sym_off > MAX_SYMBOL_OFFSET || is_noise_symbol(name) {
            continue;
        }

        if last_name == Some(name) {
            continue;
        }

        emit(format_args!(
            "  [s{:02}] {:#010x} <{}+0x{:x}> (from {:#010x})\n",
            emitted,
            candidate,
            name,
            sym_off,
            slot
        ));
        last_name = Some(name);
        emitted += 1;

        if emitted >= trace_frames.saturating_mul(2).max(walked) {
            break;
        }
    }
    if emitted == 0 {
        emit(format_args!("  <no additional symbolized entries>\n"));
    }
    */
}

fn emit_trace_entry(emit: &mut impl FnMut(fmt::Arguments<'_>), depth: usize, ret: u32) {
    if let Some((name, offset)) = kallsyms::lookup(ret.saturating_sub(1)) {
        emit(format_args!(
            "  [{:02}] {:#010x} <{}+0x{:x}>\n",
            depth, ret, name, offset
        ));
    } else {
        emit(format_args!("  [{:02}] {:#010x}\n", depth, ret));
    }
}

fn is_noise_symbol(name: &str) -> bool {
    name.contains("{{closure}}")
        || name.contains("::fmt::")
        || name.contains("core::ops::function")
        || name.contains("core::ptr::")
}

fn is_mapped_u32(addr: u32) -> bool {
    let end = match addr.checked_add(3) {
        Some(v) => v,
        None => return false,
    };

    let start_page = addr & 0xFFFF_F000;
    let end_page = end & 0xFFFF_F000;

    if page_table::get_page(start_page).is_none() {
        return false;
    }

    if start_page != end_page && page_table::get_page(end_page).is_none() {
        return false;
    }

    true
}
