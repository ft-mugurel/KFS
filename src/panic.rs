use core::panic::PanicInfo;

use crate::dump::{self, DumpStackOptions};
use crate::startup_config::logging::DEFAULT_LOG_SCREEN;
use crate::vga::text_mod::{print_fmt_on, switch_screen};
use crate::{pr_emerg, x86};

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    x86::disable_interrupts();
    pr_emerg!("KERNEL PANIC\n");
    pr_emerg!("{}\n", info);
    save_stack_trace();
    switch_screen(DEFAULT_LOG_SCREEN);
    unsafe { x86::clean_registers_and_halt() };
}

pub(crate) fn save_stack_trace() {
    pr_emerg!("Stack Trace:\n");
    dump::dump_stack_with_options(
        DumpStackOptions {
            words: 16,
            frames: 16,
            print_stack_values: false,
            scan_stack: false,
            walk_frames: false,
        },
        |args| {
            print_fmt_on(DEFAULT_LOG_SCREEN, &args);
        },
    );
}

pub(crate) fn clean_registers_and_halt() -> ! {
    unsafe { x86::clean_registers_and_halt() };
}
