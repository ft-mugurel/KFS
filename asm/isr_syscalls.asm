global isr_syscall
extern syscall_dispatcher

section .text
isr_syscall:
    pushad                      ; Save all general-purpose registers
    
    push esp                    ; Push a pointer to the saved registers
    cld                         ; Clear direction flag
    call syscall_dispatcher
    add esp, 4                  ; Clean up the pushed pointer
    
    popad                       ; Restore registers (including any modifications made by the kernel)
    iretd