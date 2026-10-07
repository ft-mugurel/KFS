ENTRY(_start)

SECTIONS {
    . = 0x08048000;

    .text : ALIGN(4) {
        *(.text._start)
        *(.text .text.*)
    }

    .rodata : ALIGN(4) {
        *(.rodata .rodata.*)
    }

    .data : ALIGN(4) {
        *(.data .data.*)
        *(.got)
        *(.got.plt)
    }

    . = ALIGN(4096);
    .bss : ALIGN(4) {
        *(.bss .bss.*)
    }

    /DISCARD/ : {
        *(.eh_frame)
        *(.comment)
        *(.note.*)
    }
}
