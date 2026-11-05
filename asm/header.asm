; header.asm
bits 32

section .multiboot
dd 0x1BADB002                   ; Magic number
dd 0x00000003                   ; Flags: align modules + request memory info
dd - (0x1BADB002 + 0x00000003)  ; Checksum

%define THREAD_SIZE 16384       ; must match sched::THREAD_SIZE

section .text
global _start
extern kmain
extern IDLE_KERNEL_STACKS         ; sched/scheduler.rs, THREAD_SIZE-aligned per-CPU stacks

_start:
    cli             ; Disable interrupts immediately to avoid pre-kmain IRQs

    ; boot on CPU 0's idle stack so current_thread_info() (esp masked down to
    ; THREAD_SIZE) finds CPU 0's ThreadInfo for the whole of kmain
    mov esp, IDLE_KERNEL_STACKS + THREAD_SIZE

    push ebx        ; multiboot info pointer
    push eax        ; multiboot magic
    call kmain      ; Jump to Rust kernel main
    
    cli
.halt_loop:
    hlt             ; Halt the CPU
    jmp .halt_loop  ; Loop if a non-maskable interrupt wakes it