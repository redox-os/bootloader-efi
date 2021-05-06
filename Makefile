export TARGET?=x86_64-unknown-uefi
export BASEDIR?=redox_bootloader

export LD=ld
export RUST_TARGET_PATH=$(CURDIR)/targets
BUILD=build/$(TARGET)

ifeq ($(TARGET),aarch64-unknown-uefi)
BOOT_EFI=efi/boot/bootaa64.efi
QEMU?=qemu-system-aarch64
QEMU_FLAGS=\
	-cpu cortex-a57 \
	-M virt \
	-m 1024 \
	-net none \
	-serial mon:stdio \
	-device virtio-gpu-pci \
	-bios /usr/share/AAVMF/AAVMF_CODE.fd
else ifeq ($(TARGET),x86_64-unknown-uefi)
BOOT_EFI=efi/boot/bootx64.efi
QEMU?=qemu-system-x86_64
QEMU_FLAGS=\
	-accel kvm \
	-M q35 \
	-m 1024 \
	-net none \
	-serial mon:stdio \
	-vga std \
	-bios /usr/share/OVMF/OVMF_CODE.fd
endif

all: $(BUILD)/boot.img

clean:
	cargo clean
	rm -rf build

update:
	git submodule update --init --recursive --remote
	cargo update

qemu: $(BUILD)/boot.img
	$(QEMU) $(QEMU_FLAGS) $<

$(BUILD)/boot.img: $(BUILD)/efi.img
	dd if=/dev/zero of=$@.tmp bs=512 count=100352
	parted $@.tmp -s -a minimal mklabel gpt
	parted $@.tmp -s -a minimal mkpart EFI FAT16 2048s 93716s
	parted $@.tmp -s -a minimal toggle 1 boot
	dd if=$< of=$@.tmp bs=512 count=98304 seek=2048 conv=notrunc
	mv $@.tmp $@

$(BUILD)/efi.img: $(BUILD)/boot.efi res/*
	dd if=/dev/zero of=$@.tmp bs=512 count=98304
	mkfs.vfat $@.tmp
	mmd -i $@.tmp efi
	mmd -i $@.tmp efi/boot
	mcopy -i $@.tmp $< ::$(BOOT_EFI)
	mv $@.tmp $@

$(BUILD)/boot.efi: Cargo.lock Cargo.toml src/* src/*/* src/*/*/*
	mkdir -p $(BUILD)
	rustup component add rust-src
	cargo rustc \
		-Z build-std=core,alloc \
		--target $(TARGET) \
		--release \
		-- \
		-C soft-float \
		--emit link=$@
