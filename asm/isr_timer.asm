global isr_timer
extern timer_interrupt_handler

section .text
isr_timer:
    ; 1. Save data segments
    push ds
    push es
    push fs
    push gs
    
    ; 2. Save general purpose registers
    pushad                      
    
    ; 3. Setup kernel data segments for the Rust handler
    ; (0x10 is your KERNEL_DATA_SELECTOR)
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    
    push esp                    ; Pass the current stack pointer to Rust
    cld                         
    call timer_interrupt_handler
    add esp, 4                  
    
    mov esp, eax                ; THE PIVOT
    
    ; 4. Restore everything from the NEW stack
    popad                       
    pop gs
    pop fs
    pop es
    pop ds
    iretd                       ; Jump to Ring 3!