global isr_lapic_timer
extern lapic_timer_interrupt_handler

section .text
isr_lapic_timer:
    push ds
    push es
    push fs
    push gs
    pushad

    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax

    push esp
    cld
    call lapic_timer_interrupt_handler
    add esp, 4

    mov esp, eax
    popad
    pop gs
    pop fs
    pop es
    pop ds
    iretd
