use core::panic::PanicInfo;

use crate::startup_config::logging::DEFAULT_LOG_SCREEN;
use crate::vga::text_mod::out::switch_screen;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
	crate::disable_interrupts();
    crate::pr_emerg!("KERNEL PANIC\n");
    crate::pr_emerg!("{}\n", info);
    save_stack_trace();
    switch_screen(DEFAULT_LOG_SCREEN);
    unsafe { crate::x86::clean_registers_and_halt() };
}

pub(crate) fn save_stack_trace() {
    crate::pr_emerg!("Stack Trace:\n");
	crate::debug::stack::dump_stack(|args| {
		crate::vga::text_mod::out::write_fmt_on(DEFAULT_LOG_SCREEN, &args);
	});
}

pub(crate) fn clean_registers_and_halt() -> ! {
	unsafe { crate::x86::clean_registers_and_halt() };
}