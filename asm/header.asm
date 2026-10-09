; header.asm
bits 32

section .multiboot
dd 0x1BADB002                   ; Magic number
dd 0x00000003                   ; Flags: align modules + request memory info
dd - (0x1BADB002 + 0x00000003)  ; Checksum

section .bss
align 16
global _stack_start
_stack_start:
    resb 16384                  ; Reserve 16 KB for the boot stack
global _stack_end
_stack_end:

section .text
global _start
extern kmain

_start:
    cli             ; Disable interrupts immediately to avoid pre-kmain IRQs

    ; setup the stack ptr
    mov esp, _stack_end

    push ebx        ; multiboot info pointer
    push eax        ; multiboot magic
    call kmain      ; Jump to Rust kernel main
    
    cli
.halt_loop:
    hlt             ; Halt the CPU
    jmp .halt_loop  ; Loop if a non-maskable interrupt wakes it