BITS 16
ORG 0x8000

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

    mov eax, [tr_cr3]          ; physical addr of the SAME page directory the BSP uses
    mov cr3, eax

    mov eax, cr0
    or eax, 0x80000000
    mov cr0, eax               ; PG=1, paging on, using existing kernel mapping

    ; this CPU's idle stack, ThreadInfo already set up by the BSP in start_ap();
    ; loaded only after PG=1 so it stays valid if the kernel moves to higher half
    mov esp, [tr_stack_top]

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
tr_stack_top:     dd 0

trampoline_end: