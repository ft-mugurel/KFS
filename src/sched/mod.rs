pub mod scheduler;
pub mod task;

use crate::gdt::gdt::{USER_CODE_SEL, USER_DATA_SEL};
use crate::paging::page_table;
use crate::paging::physical;
use crate::sched::scheduler::PROCESS_TABLE;
use crate::sched::task::{Context, ContextFrame, ProcessState, TaskStruct};
use core::mem::size_of;

static mut TEST_USER_KERNEL_STACK: [u8; 4096] = [0; 4096];

// Standard x86 User Memory Layout
pub const USER_CODE_VADDR: u32 = 0x08048000;
pub const USER_STACK_VADDR: u32 = 0xBFFFF000;

pub fn create_user_process(entry_point: fn()) -> bool {
    unsafe {
        if PROCESS_TABLE[2].is_some() {
            return false;
        }

        let k_stack_bottom = TEST_USER_KERNEL_STACK.as_ptr() as u32;
        let k_stack_top = k_stack_bottom + 4096;

        let frame_ptr = (k_stack_top - size_of::<ContextFrame>() as u32) as *mut ContextFrame;
        core::ptr::write_bytes(frame_ptr, 0, 1);

        // --- 1. Build the True Isolated Memory Space ---
        let new_cr3 =
            page_table::create_user_address_space().expect("Failed to create Address Space");
        let old_cr3 = crate::x86::read_cr3();

        crate::x86::disable_interrupts();

        crate::x86::write_cr3(new_cr3);

        let code_frame = physical::alloc_physical_page().unwrap();
        let stack_frame = physical::alloc_physical_page().unwrap();

        let user_flags =
            page_table::PAGE_PRESENT | page_table::PAGE_USER | page_table::PAGE_WRITABLE;
        page_table::map_page(USER_CODE_VADDR, code_frame, user_flags).unwrap();
        page_table::map_page(USER_STACK_VADDR - 4096, stack_frame, user_flags).unwrap();

        let code_ptr = USER_CODE_VADDR as *mut u8;
        core::ptr::copy_nonoverlapping(entry_point as *const u8, code_ptr, 1024);

        crate::x86::write_cr3(old_cr3);

        crate::x86::enable_interrupts();

        // --- 2. Setup Ring 3 Execution Frame ---
        let frame = &mut *frame_ptr;
        let user_data = (USER_DATA_SEL | 3) as u32;

        frame.ds = user_data;
        frame.es = user_data;
        frame.fs = user_data;
        frame.gs = user_data;

        // CRITICAL: Set EIP to the mapped virtual address, NOT the kernel pointer!
        frame.eip = USER_CODE_VADDR;
        frame.cs = (USER_CODE_SEL | 3) as u32;
        frame.eflags = 0x202;

        frame.user_esp = USER_STACK_VADDR - 4;
        frame.user_ss = user_data;

        let new_task = TaskStruct {
            pid: 2,
            parent_pid: 0,
            state: ProcessState::Ready,
            context: Context {
                esp: frame_ptr as u32,
                cr3: new_cr3, // The Scheduler handles the hardware CR3 swap!
            },
            kernel_stack_top: k_stack_top,
            kernel_stack_bottom: k_stack_bottom,
            tty_id: 2,
            wakeup_time: 0,
        };

        PROCESS_TABLE[2] = Some(new_task);
        crate::pr_info!("Spawned Isolated User Process PID 2\n");
        true
    }
}
