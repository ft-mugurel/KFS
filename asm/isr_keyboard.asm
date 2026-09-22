global isr_keyboard
extern keyboard_interrupt_handler

section .text
isr_keyboard:
    push ds
    push es
    push fs
    push gs
    pushad                      ; Save all 32-bit general-purpose registers

    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax

    cld                         ; Clear direction flag (C calling convention)
    call keyboard_interrupt_handler
    popad                       ; Restore all 32-bit general-purpose registers
    pop gs
    pop fs
    pop es
    pop ds
    iretd                       ; 32-bit Interrupt return