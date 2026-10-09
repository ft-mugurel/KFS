# KFS

KFS is a 32-bit x86 monolithic kernel written in Rust with a small amount of NASM assembly, designed as a bare-metal kernel from scratch. It boots through GRUB/Multiboot, initializes the protected-mode environment, sets up paging and interrupts, mounts an Ext2 filesystem, and presents a minimal interactive shell and login/user-management workflow.

This project is intentionally focused on learning kernel internals, memory management, scheduling, filesystem construction, and low-level system behavior rather than being a production-ready enterprise operating system.

## Features

- 32-bit x86 protected mode kernel, built for bare-metal execution
- Higher-half kernel mapping at 0xC0000000
- Multiboot / GRUB boot flow with ELF32 kernel loading
- Paging and virtual memory management with kernel/user separation
- Frame allocator and heap infrastructure for kernel allocations
- GDT/IDT setup, exception handling, PIC and timer initialization
- SMP bring-up via ACPI MADT and Local APIC support
- ATA/IDE disk discovery and MBR partition scanning
- Native VFS layer with Ext2 root filesystem support
- Procfs and sysfs-style pseudo-filesystems
- Multiple virtual TTYs and login prompt flow
- Minimal user-space shell (`mysh`) with built-in commands
- User accounts, credentials, and DAC-style permission checks
- PBKDF2-HMAC-SHA256 password hashing and `/etc/shadow` handling
- Kernel logging with serial/VGA output and in-kernel ring buffer
- Automated QEMU-based validation for boot and login/shell behavior

## Current scope and architecture

The kernel follows the repository’s architecture model:

- `src/kernel.rs` contains the entry point and early boot initialization
- `src/paging/` implements memory management and page mappings
- `src/gdt/` and `src/interrupts/` manage segments and interrupt handling
- `src/drivers/` and `src/fs/` handle disk discovery and filesystem integration
- `src/sched/` provides the process and scheduler model
- `src/syscalls/` handles system call dispatch
- `src/security.rs` implements authentication and access checks
- `src/tty.rs` and `src/shell/` provide the interactive terminal environment

## Build and run

This repository is built using the Makefile targets defined in the project, not by invoking Cargo directly. The build process includes assembly compilation, linker setup, symbol generation, and boot image creation.

### Required tools

- Rust nightly toolchain (pinned by `rust-toolchain.toml`)
- NASM
- GNU `ld` with ELF32 support
- GRUB rescue modules / `grub-mkrescue`
- QEMU `qemu-system-i386`
- `sfdisk` and `mkfs.ext2`
- Standard Unix build tooling (`make`, `gcc`/`clang`-compatible host toolchain)

### Standard commands

```bash
# Release build
make build

# Debug build
make build_debug

# Create the bootable ISO image
make iso

# Run headless QEMU with serial output
make run-iso-term

# Clean build outputs
make clean

# Full clean (removes build artifacts + Cargo cache)
make fclean
```

For a full boot/login verification flow, the project also includes QEMU automation:

```bash
make iso
python3 tests/test_login_shell.py
```

## Project layout

```text
.
├── src/                  # Kernel source code
├── asm/                  # NASM assembly stubs and interrupt handlers
├── user/                 # User-space toolchain / shell components
├── linker/               # Linker script and ELF layout
├── tests/                # QEMU-based validation scripts
├── build.rs              # Build-time symbol generation
├── Makefile              # Canonical build orchestration
├── Cargo.toml            # Rust crate manifest
├── rust-toolchain.toml   # Pinned nightly toolchain
├── i686-kernel.json      # Custom target file
├── README.md             # Project overview
└── .gitignore            # Git ignore rules
```

## Limitations and caveats

This kernel is a learning/research project and has deliberate practical limits:

- It targets only x86 32-bit hardware; it is not portable to ARM, x86_64, or other architectures.
- It is a monolithic kernel, not a microkernel or a general-purpose hybrid kernel.
- The filesystem support is centered on Ext2 and a minimal VFS layer; it is not a complete POSIX filesystem stack.
- Device support is intentionally limited to the drivers and block devices required by the project.
- There is no mature desktop environment, windowing system, or full GUI stack.
- Networking, USB stacks, advanced schedulers, and broad userland compatibility are not the focus of this codebase.
- The project is designed for experimentation and kernel education, not for production deployment or general-purpose computing.
- The shell and process model are intentionally minimal compared with a full Linux or BSD environment.

## Status

The repository currently represents a working educational kernel with core boot, memory, filesystem, security, TTY, and shell functionality. It is suitable for exploring operating-system design, debugging low-level hardware behavior, and validating kernel concepts in a QEMU environment.

## Contributing

Contributions are welcome if they remain aligned with the project’s goals:

- kernel correctness and architecture clarity
- educational value and maintainability
- low-level robustness and debugging support
- incremental improvements to boot, memory, filesystem, or shell functionality

Please keep changes focused and consistent with the repository’s architecture, build flow, and testing expectations described in the docs directory.

## License

This project is distributed under the repository’s existing license terms. Please review the project metadata or repository hosting page for the exact licensing details before redistributing or modifying the codebase.
