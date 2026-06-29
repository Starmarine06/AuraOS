# AuraOS – Premium Linux Base with Pure Rust Init

AuraOS is an extremely lightweight, high-performance, custom Linux operating system built from scratch. It discards heavy system management layers (like systemd) in favor of a custom, statically compiled **init system written in Rust** (`aura-init`). The user interface is a premium, translucent, glassmorphic dashboard (`aura-gui`) backed by a local AI agent daemon (`aura-agent`) running a Qwen-2.5-1.5B model.

---

## ✦ System Architecture

AuraOS is built using a multi-stage Docker environment and compiles all core OS orchestration components statically in Rust.

```mermaid
graph TD
    Kernel[Linux Kernel] -->|Launches PID 1| Init[aura-init (Rust)]
    Init -->|Initializes| Udev[eudev / Device Managers]
    Init -->|Starts| Dbus[D-Bus System Daemon]
    Init -->|Launches| Xorg[Xorg Display Server]
    Init -->|Launches| Ollama[Ollama Local LLM]
    Init -->|Launches| Agent[aura-agent Daemon (Rust)]
    Init -->|Launches| Openbox[Openbox Window Manager]
    Init -->|Launches| Picom[Picom Compositor]
    
    Xorg --> Openbox
    Picom -->|Applies Blur & Shadows| GUI[aura-gui Desktop (Rust)]
    Xorg --> Conky[Conky Desktop Widget]
    
    GUI -->|REST API Port 5050| Agent
    Agent -->|Local Socket| Ollama
```

---

## ✦ Features

### 1. Pure Rust Init System (`aura-init`)
The entire system boot process is managed by a lightweight, statically compiled Rust binary running as PID 1 (`/init`). It mounts virtual filesystems, configures network loops, initializes USB and display devices, starts services, launches Xorg, reaps zombie processes, and automatically restarts the GUI if it exits.

### 2. Translucent Glassmorphic AI GUI (`aura-gui`)
An `egui` (eframe) GUI that boots directly onto X11.
* **Frosted-Glass Transparency**: Utilizing a system-level GPU compositor (`picom`) with dual-kawase blur to create a frosted backdrop.
* **macOS/Windows Hybrid Design**: Blends circular macOS window controls (top-left traffic lights) with modern Windows Fluent-style dark theme accents.
* **Spotlight Math & App Launcher**: Typing math equations (e.g. `2+2*5`) or application names (e.g. `firefox`) into the input bar offers instant autocompletion and launching.

### 3. Local AI Agent Daemon (`aura-agent`)
A background service running on port `5050` that manages chat requests, macro capture, and system interaction.
* **System Control**: The agent has root access to run bash commands on your behalf.
* **Macro Recording**: Records physical keyboard and mouse events by listening to raw `/dev/input/event*` nodes.
* **Macro Playback**: Simulates input events using a virtual `uinput` device.
* **Bluetooth Proximity Lock**: Automatically locks the screen if your paired Bluetooth device (e.g. phone) goes out of range (using `l2ping`).

### 4. Windows Compatibility Layers
Pre-configured 32-bit (i386) and 64-bit (amd64) glibc compatibility libraries supporting:
* **Wine & Winetricks**: Run standard Windows executable binaries (`.exe`).
* **Steam**: Access your Steam gaming library natively.
* **Lutris & Bottles**: Easily manage and run Windows games and software launchers.

### 5. Rainmeter-style Desktop Widgets (`conky`)
A transparent widget rendered directly on the desktop background. Displays real-time:
* Clock and calendar.
* System uptime and kernel version.
* CPU usage graph and clock speed.
* RAM memory bar.
* Disk space usage bar.
* Loopback and ethernet network speeds.

### 6. Automated Git/GitHub Synchronization
The workspace is linked directly to your GitHub repository:
* Remote origin: `https://github.com/Starmarine06/AuraOS`
* Every verified and approved code change is automatically staged, committed, and pushed to GitHub.

---

## ✦ How to Build and Run

### Prerequisites
* Windows with **Docker Desktop** running, or any standard Linux system with Docker.

### 1. Build the Bootable ISO
From your terminal, run the build script:
```bash
./build-iso.sh
```
This script will:
1. Verify Docker is running.
2. Build the multi-stage Docker image `auraos-builder`.
3. Copy the compiled, bootable ISO file to `out/auraos.iso` in your host directory.

### 2. Run in a Virtual Machine
Create a virtual machine in **VMware Workstation** or **QEMU**:
* **OS type**: Linux / Debian 64-bit.
* **Boot mode**: UEFI or Legacy BIOS.
* **CD/DVD**: Point it to `out/auraos.iso`.
* **RAM**: Allocate at least 2GB (4GB recommended if running the LLM).

---

## ✦ How to Use AuraOS

### Chat & AI Commands
Open the translucent GUI dashboard and type a command:
* **System Check**: `"Show me my system information"` (Agent runs `uname -a`, `free -h` and responds).
* **Launch Apps**: Type `"Launch firefox"` or `"alacritty"`.
* **Lock Screen**: Pair your Bluetooth phone MAC address in Settings, enable Proximity Lock, and walk away.

### Input Macros
* To record a macro, type in the prompt: `"record macro firefox-search"`
* Perform your keyboard/mouse actions.
* To stop, click the yellow/green close/minimize window controls or type `"stop recording"`.
* To replay, type: `"play macro firefox-search"`.
