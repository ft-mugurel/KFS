ENTRY(_start)
OUTPUT_FORMAT(elf32-i386)

PHDRS {
	boot PT_LOAD;
	gdt PT_LOAD;
	text PT_LOAD;
	rodata PT_LOAD;
	init PT_LOAD;
	data PT_LOAD;
}

SECTIONS {
	. = 0M;

	.multiboot ALIGN(1K) : {
		KEEP(*(.multiboot))
	} : boot

	. = 0x00000800;

	.gdt ALIGN(8) : {
		KEEP(*(.gdt))
	} : gdt

	. = 1M;

	.text ALIGN(4K) : {
		*(.text)
		*(.text.*)
	} : text

	.rodata ALIGN(4K) : {
		*(.rodata)
		*(.rodata.*)
	} : rodata

	. = ALIGN(4K);
	__init_start = .;

	.init ALIGN(4K) : {
		*(.init.text)
		*(.init.text.*)
		*(.init.rodata)
		*(.init.rodata.*)
		. = ALIGN(16);
		__initcall_start = .;
		__initcall0_start = .;
		KEEP(*(.initcall0.init))
		KEEP(*(.initcall0s.init))
		__initcall0_end = .;
		. = ALIGN(16);
		__initcall1_start = .;
		KEEP(*(.initcall1.init))
		KEEP(*(.initcall1s.init))
		__initcall1_end = .;
		. = ALIGN(16);
		__initcall2_start = .;
		KEEP(*(.initcall2.init))
		KEEP(*(.initcall2s.init))
		__initcall2_end = .;
		. = ALIGN(16);
		__initcall3_start = .;
		KEEP(*(.initcall3.init))
		KEEP(*(.initcall3s.init))
		__initcall3_end = .;
		. = ALIGN(16);
		__initcall4_start = .;
		KEEP(*(.initcall4.init))
		KEEP(*(.initcall4s.init))
		__initcall4_end = .;
		. = ALIGN(16);
		__initcall5_start = .;
		KEEP(*(.initcall5.init))
		KEEP(*(.initcall5s.init))
		__initcall5_end = .;
		. = ALIGN(16);
		__initcall6_start = .;
		KEEP(*(.initcall6.init))
		KEEP(*(.initcall6s.init))
		__initcall6_end = .;
		. = ALIGN(16);
		__initcall7_start = .;
		KEEP(*(.initcall7.init))
		KEEP(*(.initcall7s.init))
		__initcall7_end = .;
		__initcall_end = .;
		. = ALIGN(16);
		*(.init.data)
		*(.init.data.*)
	} : init

	. = ALIGN(4K);
	__init_end = .;

	.data ALIGN(4K) : {
		*(.data)
		*(.data.*)
	} : data

	.bss ALIGN(4K) : {
		*(COMMON)
		*(.bss)
		*(.bss.*)
	} : data

	__kernel_end = .;
}
