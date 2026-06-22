global isr_syscall
extern syscall_dispatcher

section .text
isr_syscall:
    push ds
    push es
    push fs
    push gs

    pushad                      ; Save all general-purpose registers

    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax

    push esp                    ; Push a pointer to the saved registers
    cld                         ; Clear direction flag
    call syscall_dispatcher
    add esp, 4                  ; Clean up the pushed pointer
    
    popad                       ; Restore registers (including any modifications made by the kernel)
    pop gs
    pop fs
    pop es
    pop ds
    iretd