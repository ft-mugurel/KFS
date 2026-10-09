# Renk Tanımlamaları
GREY    	=	\033[030m
RED     	=	\033[031m
GREEN   	=	\033[032m
YELLOW  	=	\033[033m
BLUE    	=	\033[034m
MAGENTA 	=	\033[035m
CYAN		=	\033[036m
BOLD		=	\033[1m
RESET   	=	\033[0m

# **************************************************************************** #
# 💾 VARIABLES
# **************************************************************************** #
BUILD_DIR       = build
ISO_DIR         = $(BUILD_DIR)/iso
TARGET_DIR      = target/i686-kernel

KERNEL_BIN      = $(BUILD_DIR)/kernel.bin
KERNEL_REL_LIB  = $(TARGET_DIR)/release/libkernel.a
KERNEL_DBG_LIB  = $(TARGET_DIR)/debug/libkernel.a
KERNEL_SRCS     = $(shell find src -name '*.rs' -type f) Cargo.toml build.rs linker/linker.ld

ISO_OUT         = $(BUILD_DIR)/kernel.iso
ISO_FULL_OUT    = $(BUILD_DIR)/kernel-full.iso

ATA_DRIVE_IMG           = $(BUILD_DIR)/ata_drive.img
ATA_DRIVE_IMG_1         = $(BUILD_DIR)/ata_drive_1.img
ATA_DRIVE_IMGS          = $(ATA_DRIVE_IMG) $(ATA_DRIVE_IMG_1)
ATA_DRIVE_SIZE_MB       = 64
ATA_DRIVE_SECTORS       = $(shell expr $(ATA_DRIVE_SIZE_MB) \* 2048)
ATA_PARTITION_START     = 2048
ATA_PARTITION_SECTORS   = $(shell expr $(ATA_DRIVE_SECTORS) - $(ATA_PARTITION_START))

LINKER          = linker/linker.ld

# Automatically find ALL assembly files in the asm directory
ASM_DIR         = asm
ASM_SRCS        = $(wildcard $(ASM_DIR)/*.asm)
ASM_OBJS        = $(patsubst $(ASM_DIR)/%.asm, $(BUILD_DIR)/%.o, $(ASM_SRCS))

TRAMPOLINE_SRC = src/smp/trampoline.asm
TRAMPOLINE_BIN = $(BUILD_DIR)/trampoline.bin

USER_SHELL_DIR  = user
USER_SHELL_SRCS = $(shell find $(USER_SHELL_DIR)/src -name '*.rs') $(USER_SHELL_DIR)/Cargo.toml $(USER_SHELL_DIR)/linker.ld
USER_SHELL_BIN  = $(BUILD_DIR)/sh.bin
USER_SHELL_ELF  = $(BUILD_DIR)/user_target/i686-kernel/release/sh
FHS_STAGE       = $(BUILD_DIR)/fhs-root
FHS_STAMP       = $(BUILD_DIR)/fhs-root.stamp

# **************************************************************************** #
# 📁 DIRECTORIES
# **************************************************************************** #

GRUB_MKRESCUE	=	$(shell which grub2-mkrescue 2>/dev/null || which grub-mkrescue 2>/dev/null)
ifeq ($(GRUB_MKRESCUE),)
	$(error "grub-mkrescue not found, please install it.")
endif

GRUB_MODULE_DIR	=	$(shell [ -d /usr/lib/grub/i386-pc ] && echo /usr/lib/grub/i386-pc || ([ -d /usr/lib64/grub/i386-pc ] && echo /usr/lib64/grub/i386-pc))
ifeq ($(GRUB_MODULE_DIR),)
	$(error "GRUB i386-pc modules not found. Please install grub-pc-bin or equivalent package.")
endif

QEMU_SYSTEM	=	$(shell which qemu-system-i386 2>/dev/null || which qemu 2>/dev/null)
ifeq ($(QEMU_SYSTEM),)
	$(error "qemu-system-i386 not found, please install it.")
endif

LD		=	$(shell which ld 2>/dev/null || which ld.bfd 2>/dev/null)
ifeq ($(LD),)
	$(error "ld not found, please install it.")
endif

NASM		=	$(shell which nasm 2>/dev/null || which nasm 2>/dev/null)
ifeq ($(NASM),)
	$(error "nasm not found, please install it.")
endif

CARGO		=	$(shell which cargo 2>/dev/null)
ifeq ($(CARGO),)
	$(error "cargo not found, please install it.")
endif

RUSTC		=	$(shell which rustc 2>/dev/null)
ifeq ($(RUSTC),)
	$(error "rustc not found, please install it.")
endif

# Macros

define link_kernel
	@echo -e "$(YELLOW)[~] Linking Stage 1...$(RESET)"
	@$(LD) -m elf_i386 -T $(LINKER) -o $(KERNEL_BIN) $(ASM_OBJS) --whole-archive $(1) --no-whole-archive
	@echo -e "$(YELLOW)[~] Generating kallsyms map...$(RESET)"
	@nm -n $(KERNEL_BIN) | awk '$$2 ~ /[tTwW]/ { if ($$3 != "") print $$1 " " $$3 }' > $(BUILD_DIR)/kallsyms.tmp
	@cmp -s $(BUILD_DIR)/kallsyms.tmp $(BUILD_DIR)/kallsyms.map || mv $(BUILD_DIR)/kallsyms.tmp $(BUILD_DIR)/kallsyms.map
	@rm -f $(BUILD_DIR)/kallsyms.tmp
	@echo -e "$(YELLOW)[~] Rebuilding Rust with embedded symbols...$(RESET)"
	@$(CARGO) build $(CARGO_ARGS) $(2)
	@echo -e "$(YELLOW)[~] Linking Stage 2 (Final)...$(RESET)"
	@$(LD) -m elf_i386 -T $(LINKER) -o $(KERNEL_BIN) $(ASM_OBJS) --whole-archive $(1) --no-whole-archive
endef

# **************************************************************************** #
# 📖 RULES
# **************************************************************************** #

all: run-iso

$(BUILD_DIR)/%.o: $(ASM_DIR)/%.asm
	@mkdir -p $(BUILD_DIR)
	@$(NASM) -f elf32 $< -o $@
	@echo -e "$(CYAN)[+] NASM compiled: $<$(RESET)"

$(FHS_STAMP): $(USER_SHELL_BIN) Makefile
	@echo -e "$(YELLOW)[~] Preparing persistent FHS root image tree...$(RESET)"
	@rm -f $(FHS_STAGE)/.populated
	@mkdir -p $(FHS_STAGE)/bin $(FHS_STAGE)/boot $(FHS_STAGE)/etc $(FHS_STAGE)/home $(FHS_STAGE)/lib $(FHS_STAGE)/media $(FHS_STAGE)/mnt $(FHS_STAGE)/opt $(FHS_STAGE)/root $(FHS_STAGE)/run $(FHS_STAGE)/sbin $(FHS_STAGE)/srv $(FHS_STAGE)/dev $(FHS_STAGE)/proc $(FHS_STAGE)/sys $(FHS_STAGE)/tmp
	@mkdir -p $(FHS_STAGE)/usr/bin $(FHS_STAGE)/usr/sbin $(FHS_STAGE)/usr/lib $(FHS_STAGE)/usr/share
	@mkdir -p $(FHS_STAGE)/var/cache $(FHS_STAGE)/var/lib $(FHS_STAGE)/var/log $(FHS_STAGE)/var/spool $(FHS_STAGE)/var/tmp
	@cp $(USER_SHELL_BIN) $(FHS_STAGE)/usr/bin/mysh
	@chmod 0755 $(FHS_STAGE)/usr/bin/mysh
	@chmod 0700 $(FHS_STAGE)/root
	@printf 'root:x:0:0:root:/root:\n' > $(FHS_STAGE)/etc/passwd
	@touch $(FHS_STAGE)/etc/shadow $(FHS_STAGE)/var/log/kernel.log
	@chmod 0644 $(FHS_STAGE)/etc/passwd
	@chmod 0600 $(FHS_STAGE)/etc/shadow
	@chmod 0644 $(FHS_STAGE)/var/log/kernel.log
	@chmod 01777 $(FHS_STAGE)/tmp $(FHS_STAGE)/var/tmp
	@touch $(FHS_STAMP)
	@echo -e "$(GREEN)[✓] Persistent FHS root image tree prepared$(RESET)"

$(ATA_DRIVE_IMGS): Makefile $(FHS_STAMP)
	@mkdir -p $(BUILD_DIR)
	@echo -e "$(YELLOW)[~] Creating partitioned ATA drive image: $@...$(RESET)"
	@dd if=/dev/zero of=$@ bs=1M count=$(ATA_DRIVE_SIZE_MB) status=none
	@printf 'label: dos\nunit: sectors\n\nstart=$(ATA_PARTITION_START), size=$(ATA_PARTITION_SECTORS), type=83\n' | sfdisk --quiet $@
	@echo -e "$(YELLOW)[~] Formatting ATA partition with ext2 filesystem...$(RESET)"
	@mkfs.ext2 -q -d $(FHS_STAGE) -E offset=$$(($(ATA_PARTITION_START) * 512)) $@
	@echo -e "$(GREEN)[✓] ATA drive image created: $@$(RESET)"

$(TRAMPOLINE_BIN): $(TRAMPOLINE_SRC)
	@mkdir -p $(BUILD_DIR)
	@echo -e "$(YELLOW)[~] Compiling trampoline.asm...$(RESET)"
	@nasm -f bin $(TRAMPOLINE_SRC) -o $(TRAMPOLINE_BIN)

$(USER_SHELL_BIN): $(USER_SHELL_SRCS)
	@mkdir -p $(BUILD_DIR)
	@echo -e "$(YELLOW)[~] Building user-space shell in Rust...$(RESET)"
	@RUSTFLAGS="-C link-arg=-T$(abspath $(USER_SHELL_DIR)/linker.ld)" $(CARGO) build --release --manifest-path $(USER_SHELL_DIR)/Cargo.toml -Zjson-target-spec --target $(abspath i686-kernel.json) --target-dir $(BUILD_DIR)/user_target
	@objcopy -O binary $(USER_SHELL_ELF) $@
	@echo -e "$(GREEN)[✓] User shell binary built: $@$(RESET)"

build: CARGO_ARGS = --no-default-features -Zjson-target-spec
build: $(ASM_OBJS) $(TRAMPOLINE_BIN) $(USER_SHELL_BIN) $(KERNEL_SRCS) $(LINKER)
	@echo -e "$(BOLD)$(CYAN)[~] Building Release Kernel...$(RESET)"
	@$(CARGO) build --release -Zjson-target-spec
	$(call link_kernel, $(KERNEL_REL_LIB), --release)
	@echo -e "$(BOLD)$(GREEN)[✓] RELEASE KERNEL BUILD DONE$(RESET)"

build_debug: CARGO_ARGS = -Zjson-target-spec
build_debug: $(ASM_OBJS) $(TRAMPOLINE_BIN) $(USER_SHELL_BIN) $(KERNEL_SRCS) $(LINKER)
	@echo -e "$(BOLD)$(YELLOW)[~] Building Debug Kernel...$(RESET)"
	@$(CARGO) build $(CARGO_ARGS)
	$(call link_kernel, $(KERNEL_DBG_LIB), )
	@echo -e "$(BOLD)$(GREEN)[✓] DEBUG KERNEL BUILD DONE$(RESET)"

# Reusable ISO preparation step
$(ISO_DIR)/boot/grub/grub.cfg: grub/grub.cfg
	@mkdir -p $(ISO_DIR)/boot/grub
	@cp grub/grub.cfg $(ISO_DIR)/boot/grub/

run: build
	@$(QEMU_SYSTEM) -kernel $(KERNEL_BIN) -monitor stdio
	@echo -e "\n$(BOLD)$(CYAN)[✓] KERNEL EXIT DONE$(RESET)"

debug: build_debug
	@$(QEMU_SYSTEM) -kernel $(KERNEL_BIN) -s -S &
	@gdb -x .gdbinit
	@echo -e "\n$(BOLD)$(CYAN)[✓] KERNEL DEBUG EXIT DONE$(RESET)"

iso: build $(ISO_DIR)/boot/grub/grub.cfg
	@cp $(KERNEL_BIN) $(ISO_DIR)/boot/
	@$(GRUB_MKRESCUE) -o $(ISO_OUT) $(ISO_DIR) --directory=$(GRUB_MODULE_DIR) \
		--modules="multiboot" --locales="" --fonts="" --themes="" 2>/dev/null
	@echo -e "$(BOLD)$(GREEN)[✓] ISO BUILD DONE: $(ISO_OUT)$(RESET)"

iso-full: build $(ISO_DIR)/boot/grub/grub.cfg
	@cp $(KERNEL_BIN) $(ISO_DIR)/boot/
	@$(GRUB_MKRESCUE) -o $(ISO_FULL_OUT) $(ISO_DIR) --directory=$(GRUB_MODULE_DIR) --modules="multiboot" 2>/dev/null
	@echo -e "$(BOLD)$(GREEN)[✓] FULL ISO BUILD DONE: $(ISO_FULL_OUT)$(RESET)"

run-iso: iso $(ATA_DRIVE_IMGS)
	@$(QEMU_SYSTEM) -m 4G \
		-drive file=$(ATA_DRIVE_IMG),format=raw,if=ide,index=0,media=disk \
		-drive file=$(ATA_DRIVE_IMG_1),format=raw,if=ide,index=1,media=disk \
		-drive format=raw,file=$(ISO_OUT),media=cdrom \
		-d cpu_reset,guest_errors -no-reboot -no-shutdown \
		-serial stdio \
		-smp 8 \
		-accel kvm \
		-boot order=d
	@echo -e "\n$(BOLD)$(CYAN)[✓] QEMU EXIT DONE$(RESET)"

run-iso-full: iso-full $(ATA_DRIVE_IMGS)
	@$(QEMU_SYSTEM) -m 4G \
		-drive file=$(ATA_DRIVE_IMG),format=raw,if=ide,index=0,media=disk \
		-drive file=$(ATA_DRIVE_IMG_1),format=raw,if=ide,index=1,media=disk \
		-drive format=raw,file=$(ISO_FULL_OUT),media=cdrom \
		-boot order=d
	@echo -e "\n$(BOLD)$(CYAN)[✓] QEMU EXIT DONE$(RESET)"

run-iso-term: iso $(ATA_DRIVE_IMGS)
	@$(QEMU_SYSTEM) -m 4G \
		-drive file=$(ATA_DRIVE_IMG),format=raw,if=ide,index=0,media=disk \
		-drive file=$(ATA_DRIVE_IMG_1),format=raw,if=ide,index=1,media=disk \
		-drive format=raw,file=$(ISO_OUT),media=cdrom \
		-boot order=d -nographic
	@echo -e "\n$(BOLD)$(CYAN)[✓] QEMU EXIT DONE$(RESET)"

test-chaos: iso
	@python3 tests/test_chaos.py

clean:
	@rm -rf $(BUILD_DIR)/
	@echo -e "$(BOLD)$(RED)[♻︎] DELETED BUILD ARTIFACTS$(RESET)"

fclean: clean
	@$(CARGO) clean
	@echo -e "$(BOLD)$(RED)[♻︎] DELETED CARGO TARGETS$(RESET)"

re: clean all

.PHONY: all build build_debug iso iso-full run debug run-iso run-iso-full run-iso-term test-chaos clean fclean re