# Stage 1: Build Rust Workspace Statically
FROM rust:latest AS builder
RUN apt-get update && apt-get install -y musl-tools
RUN rustup target add x86_64-unknown-linux-musl

WORKDIR /app
COPY . .
# Compile all workspace members statically in release mode
RUN cargo build --release --target x86_64-unknown-linux-musl

# Stage 1.5: Build OpenClaw Node Gateway
FROM node:22-alpine AS openclaw-builder
WORKDIR /app
COPY openclaw /app
RUN npm install -g pnpm
RUN pnpm install --store-dir /pnpm-store
RUN OPENCLAW_RUN_NODE_SKIP_DTS_BUILD=1 OPENCLAW_TSDOWN_MAX_OLD_SPACE_MB=3072 node scripts/build-all.mjs qaRuntime

# Stage 2: Create Custom Rootfs and Packaging Environment
FROM debian:bookworm-slim AS packager
ENV DEBIAN_FRONTEND=noninteractive

# Enable contrib/non-free repos for Steam & firmware
RUN sed -i 's/Components: main/Components: main contrib non-free non-free-firmware/g' /etc/apt/sources.list.d/debian.sources

# Enable multiarch for 32-bit packages (required for 32-bit Wine & Steam)
RUN dpkg --add-architecture i386

# Install build dependencies for ISO generation and target packages
# Using syslinux/isolinux instead of GRUB (same as Arch Linux ISOs)
RUN apt-get update && apt-get install -y \
    xorriso \
    syslinux \
    syslinux-common \
    isolinux \
    mtools \
    squashfs-tools \
    cpio \
    gzip \
    zstd \
    curl \
    live-boot \
    live-boot-initramfs-tools \
    initramfs-tools \
    linux-image-amd64 \
    systemd-resolved \
    && rm -rf /var/lib/apt/lists/*

# Create a target rootfs directory structure
RUN mkdir -p /rootfs && \
    mkdir -p /rootfs/bin /rootfs/sbin /rootfs/etc /rootfs/proc /rootfs/sys /rootfs/dev /rootfs/run /rootfs/tmp \
    /rootfs/usr/bin /rootfs/usr/sbin /rootfs/lib /rootfs/lib64 /rootfs/opt /rootfs/var/lib/aura /rootfs/home/aura \
    /rootfs/var/lib/dpkg && touch /rootfs/var/lib/dpkg/status

# Install system libraries and user applications inside the target rootfs
RUN apt-get update && apt-get install -y --install-recommends --no-install-suggests \
    -o Root="/rootfs" \
    libc6 \
    dash \
    bash \
    coreutils \
    debianutils \
    grep \
    sed \
    tar \
    gzip \
    findutils \
    udev \
    dbus \
    dbus-x11 \
    kmod \
    iproute2 \
    bluez \
    bluez-tools \
    pulseaudio \
    pavucontrol \
    xserver-xorg-core \
    xserver-xorg-video-fbdev \
    xserver-xorg-video-all \
    openbox \
    picom \
    conky-all \
    alacritty \
    firefox-esr \
    wine \
    wine64 \
    wine32:i386 \
    winetricks \
    steam \
    lutris \
    libgl1-mesa-dri \
    libgl1-mesa-dri:i386 \
    mesa-vulkan-drivers \
    mesa-vulkan-drivers:i386 \
    && rm -rf /var/lib/apt/lists/*

# Manually configure symlinks for essential interpreters in the target rootfs
RUN ln -sf dash /rootfs/bin/sh && ln -sf bash /rootfs/bin/bash

# Download and extract standalone Linux Ollama binary into target rootfs
RUN curl -L https://ollama.com/download/ollama-linux-amd64.tar.zst | zstd -d | (tar -x -C /rootfs/usr/bin --strip-components=1 bin/ollama || true) && \
    chmod +x /rootfs/usr/bin/ollama

# Copy static Rust binaries into target rootfs
COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/aura-init /rootfs/init
COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/aura-agent /rootfs/usr/bin/aura-agent
COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/aura-gui /rootfs/usr/bin/aura-gui
RUN chmod +x /rootfs/init /rootfs/usr/bin/aura-agent /rootfs/usr/bin/aura-gui

# Copy picom and conky configurations into target rootfs
COPY picom.conf /rootfs/etc/picom.conf
COPY conky.conf /rootfs/etc/conky.conf

# Setup default user 'aura' inside target rootfs
RUN echo "aura:x:1000:1000:Aura User:/home/aura:/bin/bash" >> /rootfs/etc/passwd && \
    echo "aura:x:1000:" >> /rootfs/etc/group && \
    echo "aura:!::" >> /rootfs/etc/shadow && \
    chown -R 1000:1000 /rootfs/home/aura

# Copy wallpapers into target rootfs with proper ownership
COPY --chown=1000:1000 backgrounds /rootfs/usr/share/backgrounds/aura
RUN mkdir -p /rootfs/var/lib/ollama && chown -R 1000:1000 /rootfs/var/lib/ollama

# Copy built OpenClaw Node gateway from builder stage with proper ownership
COPY --chown=1000:1000 --from=openclaw-builder /app /rootfs/opt/openclaw

# Prepare the ISO build layout (Arch-style: isolinux for BIOS boot)
WORKDIR /iso
RUN mkdir -p isolinux live

# Compress target rootfs into SquashFS inside /iso/live
RUN mksquashfs /rootfs /iso/live/filesystem.squashfs -comp xz -noappend

# Copy kernel and initrd
RUN cp $(ls /boot/vmlinuz-* | head -n 1) /iso/live/vmlinuz && \
    cp $(ls /boot/initrd.img-* | head -n 1) /iso/live/initrd.img

# Copy isolinux boot files (same as Arch Linux ISO approach)
RUN cp /usr/lib/ISOLINUX/isolinux.bin /iso/isolinux/ && \
    cp /usr/lib/syslinux/modules/bios/ldlinux.c32 /iso/isolinux/ && \
    cp /usr/lib/syslinux/modules/bios/menu.c32 /iso/isolinux/ && \
    cp /usr/lib/syslinux/modules/bios/libutil.c32 /iso/isolinux/

# Create isolinux boot config
# init=/init: tells live-boot to exec our Rust binary after pivot_root
# nouveau.modeset=0: prevents nouveau from crashing without NVIDIA firmware
RUN printf 'UI menu.c32\nPROMPT 0\nTIMEOUT 30\n\nLABEL auraos\n  MENU LABEL AuraOS\n  KERNEL /live/vmlinuz\n  APPEND initrd=/live/initrd.img boot=live quiet splash init=/init nouveau.modeset=0\n\nLABEL auraos-safe\n  MENU LABEL AuraOS (safe video)\n  KERNEL /live/vmlinuz\n  APPEND initrd=/live/initrd.img boot=live quiet splash init=/init nomodeset nouveau.modeset=0\n' > /iso/isolinux/isolinux.cfg

# Copy pre-cached Ollama model files
COPY --chown=1000:1000 ollama-cache /iso

# Generate hybrid ISO with isolinux boot sector (Arch-style)
RUN mkdir -p /out
CMD xorriso -as mkisofs \
    -o /out/auraos.iso \
    -b isolinux/isolinux.bin \
    -c isolinux/boot.cat \
    -no-emul-boot \
    -boot-load-size 4 \
    -boot-info-table \
    -isohybrid-mbr /usr/lib/ISOLINUX/isohdpfx.bin \
    -V AURAOS \
    /iso
