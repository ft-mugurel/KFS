global isr_syscall
extern syscall_dispatcher

section .text
isr_syscall:
    ;push 0       Dummy error code to align ContextFrame
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

    ; syscall_dispatcher now returns:
    ;   - EAX = 0       if normal syscall (no reschedule needed)
    ;   - EAX = non-zero if context switch needed (next task's ESP)
    cmp eax, 0
    je .normal_return

    ; Context switch path: dispatcher returned next ESP in EAX
    mov esp, eax

.normal_return:
    popad                       ; Restore registers (including any modifications made by the kernel)
    pop gs
    pop fs
    pop es
    pop ds
    iretd