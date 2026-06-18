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

ISO_OUT         = $(BUILD_DIR)/kernel.iso
ISO_FULL_OUT    = $(BUILD_DIR)/kernel-full.iso

LINKER          = linker/linker.ld

# Automatically find ALL assembly files in the asm directory
ASM_DIR         = asm
ASM_SRCS        = $(wildcard $(ASM_DIR)/*.asm)
ASM_OBJS        = $(patsubst $(ASM_DIR)/%.asm, $(BUILD_DIR)/%.o, $(ASM_SRCS))

# **************************************************************************** #
# 
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
	@$(LD) -m elf_i386 -T $(LINKER) -o $(KERNEL_BIN) $(ASM_OBJS) $(1)
	@echo -e "$(YELLOW)[~] Generating kallsyms map...$(RESET)"
	@nm -n $(KERNEL_BIN) | awk '$$2 ~ /[tTwW]/ { if ($$3 != "") print $$1 " " $$3 }' > $(BUILD_DIR)/kallsyms.map
	@echo -e "$(YELLOW)[~] Rebuilding Rust with embedded symbols...$(RESET)"
	@$(CARGO) build $(CARGO_ARGS) $(2)
	@echo -e "$(YELLOW)[~] Linking Stage 2 (Final)...$(RESET)"
	@$(LD) -m elf_i386 -T $(LINKER) -o $(KERNEL_BIN) $(ASM_OBJS) $(1)
endef

# **************************************************************************** #
# 📖 RULES
# **************************************************************************** #

all: run-iso

$(BUILD_DIR)/%.o: $(ASM_DIR)/%.asm
	@mkdir -p $(BUILD_DIR)
	@$(NASM) -f elf32 $< -o $@
	@echo -e "$(CYAN)[+] NASM compiled: $<$(RESET)"

build: CARGO_ARGS = --no-default-features
build: $(ASM_OBJS)
	@echo -e "$(BOLD)$(CYAN)[~] Building Release Kernel...$(RESET)"
	@$(CARGO) build --release
	$(call link_kernel, $(KERNEL_REL_LIB), --release)
	@echo -e "$(BOLD)$(GREEN)[✓] RELEASE KERNEL BUILD DONE$(RESET)"

build_debug: $(ASM_OBJS)
	@echo -e "$(BOLD)$(YELLOW)[~] Building Debug Kernel...$(RESET)"
	@$(CARGO) build
	$(call link_kernel, $(KERNEL_DBG_LIB), )
	@echo -e "$(BOLD)$(GREEN)[✓] DEBUG KERNEL BUILD DONE$(RESET)"

# Reusable ISO preparation step
$(ISO_DIR)/boot/grub/grub.cfg:
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

run-iso: iso
	@$(QEMU_SYSTEM) -m 4G -drive format=raw,file=$(ISO_OUT),media=cdrom -boot order=d
	@echo -e "\n$(BOLD)$(CYAN)[✓] QEMU EXIT DONE$(RESET)"

run-iso-full: iso-full
	@$(QEMU_SYSTEM) -m 4G -drive format=raw,file=$(ISO_FULL_OUT),media=cdrom -boot order=d
	@echo -e "\n$(BOLD)$(CYAN)[✓] QEMU EXIT DONE$(RESET)"

run-iso-term: iso
	@$(QEMU_SYSTEM) -m 4G \
		-drive format=raw,file=$(ISO_OUT),media=cdrom \
		-boot order=d -nographic
	@echo -e "\n$(BOLD)$(CYAN)[✓] QEMU EXIT DONE$(RESET)"

clean:
	@rm -rf $(BUILD_DIR)/
	@echo -e "$(BOLD)$(RED)[♻︎] DELETED BUILD ARTIFACTS$(RESET)"

fclean: clean
	@$(CARGO) clean
	@echo -e "$(BOLD)$(RED)[♻︎] DELETED CARGO TARGETS$(RESET)"

re: clean all

.PHONY: all build build_debug iso iso-full run debug run-iso run-iso-full run-iso-term clean fclean re