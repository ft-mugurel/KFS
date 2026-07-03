use super::{ContextFrame, ProcessState, TaskStruct, EMPTY_VMA, MAX_VMAS, PROCESS_TABLE};
use crate::gdt::{USER_CODE_SEL, USER_DATA_SEL};
use crate::{paging, pr_err, pr_info, x86};
use core::mem::{size_of, MaybeUninit};
use core::ptr::{copy_nonoverlapping, write_bytes};

const USER_CODE_VADDR: u32 = 0x08048000;
const USER_DATA_VADDR: u32 = 0x0804A000;
const USER_BSS_VADDR: u32 = 0x0804B000;
const USER_STACK_VADDR: u32 = 0xBFFFF000;

pub unsafe fn create_user_process(entry_point: unsafe fn(), entry_size: usize) -> bool {
    let tty_id = 1;
    if PROCESS_TABLE[tty_id].is_some() {
        return false;
    }
    let k_stack_frame = match paging::alloc_physical_page() {
        Ok(frame) => frame,
        Err(e) => {
            pr_err!(
                "Failed to allocate kernel stack frame for user process: {:?}\n",
                e
            );
            return false;
        }
    };
    let k_stack_bottom = k_stack_frame;
    let k_stack_top = k_stack_bottom + 4096;

    let frame_ptr = (k_stack_top - size_of::<ContextFrame>() as u32) as *mut ContextFrame;
    write_bytes(frame_ptr, 0, 1);

    let old_cr3 = x86::read_cr3();
    let new_cr3 = match paging::clone_address_space(old_cr3) {
        Ok(cr3) => cr3,
        Err(e) => {
            pr_err!("Failed to create address space for user process: {:?}\n", e);
            return false;
        }
    };

    x86::disable_interrupts();

    x86::write_cr3(new_cr3);

    let code_frame = paging::alloc_physical_page().unwrap();
    let stack_frame = paging::alloc_physical_page().unwrap();

    let user_flags = paging::PAGE_PRESENT | paging::PAGE_USER | paging::PAGE_WRITABLE;
    paging::map_page(USER_CODE_VADDR, code_frame, user_flags).unwrap();
    paging::map_page(USER_STACK_VADDR - 4096, stack_frame, user_flags).unwrap();
    let data_frame = paging::alloc_physical_page().unwrap();
    paging::map_page(USER_DATA_VADDR, data_frame, user_flags).unwrap();

    // map and zero the BSS Sector
    let bss_frame = paging::alloc_physical_page().unwrap();
    paging::map_page(USER_BSS_VADDR, bss_frame, user_flags).unwrap();
    write_bytes(USER_BSS_VADDR as *mut u8, 0, 4096);

    let code_ptr = USER_CODE_VADDR as *mut u8;
    copy_nonoverlapping(entry_point as *const u8, code_ptr, entry_size);

    // exit even if the user didn't call exit, to avoid returning to the kernel
    let trampoline_vaddr = USER_CODE_VADDR + 0x800;
    let trampoline_ptr = trampoline_vaddr as *mut u8;
    let exit_payload: [u8; 12] = [
        0xB8, 0x01, 0x00, 0x00, 0x00, // mov eax, 1
        0xBB, 0x00, 0x00, 0x00, 0x00, // mov ebx, 0
        0xCD, 0x80, // int 0x80
    ];
    copy_nonoverlapping(exit_payload.as_ptr(), trampoline_ptr, 12);

    let stack_top_ptr = (USER_STACK_VADDR - 4) as *mut u32;
    stack_top_ptr.write(trampoline_vaddr);

    x86::write_cr3(old_cr3);

    x86::enable_interrupts();

    // Ring 3 Execution Frame
    let frame = &mut *frame_ptr;
    let user_data = (USER_DATA_SEL | 3) as u32;

    frame.ds = user_data;
    frame.es = user_data;
    frame.fs = user_data;
    frame.gs = user_data;

    frame.eip = USER_CODE_VADDR;
    frame.cs = (USER_CODE_SEL | 3) as u32;
    frame.eflags = 0x202;

    frame.user_esp = USER_STACK_VADDR - 4;
    frame.user_ss = user_data;

    let mut new_task: TaskStruct = MaybeUninit::zeroed().assume_init();
    new_task.pid = 2;
    new_task.uid = 1000;
    new_task.state = ProcessState::Ready;
    new_task.context.esp = frame_ptr as u32;
    new_task.context.cr3 = new_cr3;

    // Memory Tracking Initialization
    new_task.memory.code_base = USER_CODE_VADDR;
    new_task.memory.code_size = 4096;
    new_task.memory.data_base = USER_DATA_VADDR;
    new_task.memory.data_size = 4096;
    new_task.memory.bss_base = USER_BSS_VADDR;
    new_task.memory.bss_size = 4096;
    new_task.memory.stack_base = USER_STACK_VADDR;
    new_task.memory.stack_limit = USER_STACK_VADDR - 4096;
    new_task.memory.heap_base = 0x4000_0000;
    new_task.memory.heap_brk = 0x4000_0000;
    new_task.memory.vmas = [EMPTY_VMA; MAX_VMAS];

    new_task.kernel_stack_top = k_stack_top;
    new_task.kernel_stack_bottom = k_stack_bottom;

    PROCESS_TABLE[tty_id] = Some(new_task);
    pr_info!("Spawned Isolated User Process PID {}\n", tty_id);
    true
}
