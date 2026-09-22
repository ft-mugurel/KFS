extern exception_common_handler

global restore_context_and_iret
restore_context_and_iret:
    mov esp, [esp + 4]          ; load new ESP from argument
    popad
    pop gs
    pop fs
    pop es
    pop ds
    iretd

%macro EXC_NOERR 1
global isr_exception_%1
isr_exception_%1:
    push 0                      ; 1. Push dummy error code to align stack frame
    push ds
    push es
    push fs
    push gs
    pushad                      ; 2. Save registers
    
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    
    push esp                    ; 3. ARG 2: Pointer to Registers struct
    push dword %1               ; 4. ARG 1: Vector number
    
    cld
    call exception_common_handler
    
    add esp, 8                  ; 5. Clean up the 2 arguments (8 bytes)
    popad                       ; 6. Restore registers
    pop gs
    pop fs
    pop es
    pop ds
    add esp, 4                  ; 7. Clean up the dummy error code
    iretd                       ; 8. 32-bit return
%endmacro

%macro EXC_ERR 1
global isr_exception_%1
isr_exception_%1:
    ; The CPU has already pushed a 4-byte error code here
    push ds
    push es
    push fs
    push gs
    pushad                      ; 1. Save registers
    
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    
    push esp                    ; 2. ARG 2: Pointer to Registers struct
    push dword %1               ; 3. ARG 1: Vector number
    
    cld
    call exception_common_handler
    
    add esp, 8                  ; 4. Clean up the 2 arguments (8 bytes)
    popad                       ; 5. Restore registers
    pop gs
    pop fs
    pop es
    pop ds
    add esp, 4                  ; 6. Clean up the CPU's error code
    iretd                       ; 7. 32-bit return
%endmacro

; ----------------
; Exception Hooks
; ----------------
EXC_NOERR 0
EXC_NOERR 1
EXC_NOERR 2
EXC_NOERR 3
EXC_NOERR 4
EXC_NOERR 5
EXC_NOERR 6
EXC_NOERR 7
EXC_ERR 8
EXC_NOERR 9
EXC_ERR 10
EXC_ERR 11
EXC_ERR 12
EXC_ERR 13
EXC_ERR 14
EXC_NOERR 15
EXC_NOERR 16
EXC_ERR 17
EXC_NOERR 18
EXC_NOERR 19
EXC_NOERR 20
EXC_NOERR 21
EXC_NOERR 22
EXC_NOERR 23
EXC_NOERR 24
EXC_NOERR 25
EXC_NOERR 26
EXC_NOERR 27
EXC_NOERR 28
EXC_NOERR 29
EXC_NOERR 30
EXC_NOERR 31