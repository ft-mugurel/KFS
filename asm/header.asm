; header.asm
bits 32

section .multiboot
dd 0x1BADB002                   ; Magic number
dd 0x00000003                   ; Flags: align modules + request memory info
dd - (0x1BADB002 + 0x00000003)  ; Checksum

section .text
global _start
extern kmain

_start:
    cli             ; Disable interrupts immediately to avoid pre-kmain IRQs
    
    push ebx        ; multiboot info pointer
    push eax        ; multiboot magic
    call kmain      ; Jump to Rust kernel main
    
    cli
.halt_loop:
    hlt             ; Halt the CPU
    jmp .halt_loop  ; Loop if a non-maskable interrupt wakes it