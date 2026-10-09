use core::panic::PanicInfo;

use crate::dump::{self, DumpStackOptions};
use crate::vga::text_mod::{active_screen_index, print_fmt_on};
use crate::{pr_emerg, x86};

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    x86::disable_interrupts();
    pr_emerg!("KERNEL PANIC\n");
    pr_emerg!("{}\n", info);
    save_stack_trace();
    unsafe { x86::clean_registers_and_halt() };
}

pub(crate) fn save_stack_trace() {
    pr_emerg!("Stack Trace:\n");
    let screen = active_screen_index();
    dump::dump_stack_with_options(
        DumpStackOptions {
            words: 16,
            frames: 16,
            print_stack_values: false,
            scan_stack: true,
            walk_frames: true,
        },
        |args| {
            print_fmt_on(screen, &args);
        },
    );
}

pub(crate) fn clean_registers_and_halt() -> ! {
    unsafe { x86::clean_registers_and_halt() };
}
