BITS 16
ORG 0x8000

%define MAX_CPUS 8
%define STACK_SIZE 4096

trampoline_start:
    cli
    xor ax, ax
    mov ds, ax
    mov es, ax
    mov ss, ax

    lgdt [tr_gdt_ptr]          ; reuse the SAME GDT the BSP built (gdt.rs), physical addr fits in 32 bits fine since we're about to go protected

    mov eax, cr0
    or eax, 1
    mov cr0, eax               ; PE=1

    jmp 0x08:trampoline_pm32   ; far jump, selector 0x08 = KERNEL_CODE_SEL, flushes prefetch/CS

BITS 32
trampoline_pm32:
    mov ax, 0x10               ; KERNEL_DATA_SEL
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    mov ss, ax

    ; temporary stack, just enough to call into Rust — real per-cpu stack gets
    ; swapped in immediately inside ap_entry()
    mov eax, [tr_cpu_id]
    inc eax
    imul eax, eax, STACK_SIZE
    add eax, tr_tmp_stack
    mov esp, eax

    mov eax, [tr_cr3]          ; physical addr of the SAME page directory the BSP uses
    mov cr3, eax

    mov eax, cr0
    or eax, 0x80000000
    mov cr0, eax               ; PG=1 — paging on, using existing kernel mapping

    mov eax, [tr_cpu_id]       ; which slot in acpi cpu list this core corresponds to
    push eax
    call [tr_ap_entry_addr]    ; extern "C" fn ap_entry(cpu_id: u32) -> ! in Rust, never returns

.hang:
    hlt
    jmp .hang

TIMES 0x100 - ($ - trampoline_start) db 0   ; pad code to a fixed 256-byte boundary
tr_data_start:
tr_gdt_ptr:       dw 0            ; filled in at runtime: same GdtPointer as gdt.rs uses
                  dd 0
tr_cr3:           dd 0
tr_cpu_id:        dd 0
tr_ap_entry_addr: dd 0
tr_tmp_stack:     times (MAX_CPUS * STACK_SIZE) db 0
tr_tmp_stack_top:

trampoline_end: