global isr_keyboard
extern keyboard_interrupt_handler

section .text
isr_keyboard:
    pushad                      ; Save all 32-bit general-purpose registers
    cld                         ; Clear direction flag (C calling convention)
    call keyboard_interrupt_handler
    popad                       ; Restore all 32-bit general-purpose registers
    iretd                       ; 32-bit Interrupt return