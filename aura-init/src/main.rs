use std::process::Command;
use std::thread;
use std::time::Duration;
use std::fs;
use std::path::Path;

fn main() {
    println!("========================================");
    println!("        Welcome to AuraOS Init          ");
    println!("        Statically Compiled in Rust     ");
    println!("========================================");

    // 1. Mount essential filesystems
    if let Err(e) = mount_fs("proc", "/proc", "proc", 0, None) {
        eprintln!("Failed to mount /proc: {}", e);
    }
    if let Err(e) = mount_fs("sysfs", "/sys", "sysfs", 0, None) {
        eprintln!("Failed to mount /sys: {}", e);
    }
    if let Err(e) = mount_fs("devtmpfs", "/dev", "devtmpfs", 0, None) {
        eprintln!("Failed to mount /dev: {}", e);
    }
    
    // Create folders for run, tmp, dev/pts, dev/shm
    let _ = fs::create_dir_all("/run");
    let _ = fs::create_dir_all("/tmp");
    let _ = fs::create_dir_all("/dev/pts");
    let _ = fs::create_dir_all("/dev/shm");

    if let Err(e) = mount_fs("tmpfs", "/run", "tmpfs", 0, None) {
        eprintln!("Failed to mount /run: {}", e);
    }
    if let Err(e) = mount_fs("tmpfs", "/tmp", "tmpfs", 0, None) {
        eprintln!("Failed to mount /tmp: {}", e);
    }
    if let Err(e) = mount_fs("devpts", "/dev/pts", "devpts", 0, None) {
        eprintln!("Failed to mount /dev/pts: {}", e);
    }
    if let Err(e) = mount_fs("tmpfs", "/dev/shm", "tmpfs", 0, None) {
        eprintln!("Failed to mount /dev/shm: {}", e);
    }

    // 2. Set environment variables
    std::env::set_var("PATH", "/bin:/sbin:/usr/bin:/usr/sbin");
    std::env::set_var("HOME", "/home/aura");

    // Set hostname
    let _ = fs::write("/proc/sys/kernel/hostname", "auraos");

    // 3. Start udev daemon
    println!("Starting udevd...");
    if let Err(e) = Command::new("/lib/systemd/systemd-udevd")
        .arg("--daemon")
        .spawn()
        .or_else(|_| Command::new("/sbin/udevd").arg("--daemon").spawn())
    {
        eprintln!("Failed to start udevd: {}", e);
    }
    
    // Trigger and settle device events
    thread::sleep(Duration::from_millis(500));
    let _ = Command::new("udevadm").args(&["trigger", "--action=add"]).status();
    let _ = Command::new("udevadm").arg("settle").status();

    // 4. Setup loopback interface
    println!("Setting up loopback interface...");
    let _ = Command::new("ip").args(&["link", "set", "up", "dev", "lo"]).status();

    // 5. Mount the ISO (to access models)
    println!("Scanning for AuraOS ISO volume...");
    let _ = fs::create_dir_all("/media/iso");
    let mut mounted = false;
    for _ in 0..10 {
        if Path::new("/dev/disk/by-label/AURAOS").exists() {
            if mount_fs("/dev/disk/by-label/AURAOS", "/media/iso", "iso9660", 0, None).is_ok() {
                println!("Successfully mounted AuraOS ISO to /media/iso");
                mounted = true;
                break;
            } else if mount_fs("/dev/disk/by-label/AURAOS", "/media/iso", "vfat", 0, None).is_ok() {
                println!("Successfully mounted AuraOS partition (FAT) to /media/iso");
                mounted = true;
                break;
            }
        }
        thread::sleep(Duration::from_millis(500));
    }

    if !mounted {
        println!("Label AURAOS not found or mount failed. Scanning all block devices...");
        if let Ok(entries) = fs::read_dir("/sys/class/block") {
            for entry in entries.flatten() {
                let dev_name = entry.file_name().to_string_lossy().into_owned();
                if dev_name.starts_with("sr") || dev_name.starts_with("sd") || dev_name.starts_with("vd") {
                    let dev_path = format!("/dev/{}", dev_name);
                    if mount_fs(&dev_path, "/media/iso", "iso9660", 0, None).is_ok() {
                        if Path::new("/media/iso/models").exists() {
                            println!("Successfully mounted AuraOS media on {} via fallback", dev_path);
                            mounted = true;
                            break;
                        }
                        // Unmount if not ours
                        unsafe {
                            let c_target = std::ffi::CString::new("/media/iso").unwrap();
                            libc::umount(c_target.as_ptr());
                        }
                    } else if mount_fs(&dev_path, "/media/iso", "vfat", 0, None).is_ok() {
                        if Path::new("/media/iso/models").exists() {
                            println!("Successfully mounted AuraOS media (FAT) on {} via fallback", dev_path);
                            mounted = true;
                            break;
                        }
                        // Unmount if not ours
                        unsafe {
                            let c_target = std::ffi::CString::new("/media/iso").unwrap();
                            libc::umount(c_target.as_ptr());
                        }
                    }
                }
            }
        }
    }

    // Link models directory if found
    let _ = fs::create_dir_all("/var/lib/ollama");
    let target_ollama = "/var/lib/ollama/.ollama";
    let _ = fs::remove_dir_all(target_ollama);
    let _ = fs::remove_file(target_ollama);

    let mut model_source = None;
    if Path::new("/run/live/medium/.ollama").exists() {
        model_source = Some("/run/live/medium/.ollama");
    } else if mounted && Path::new("/media/iso/.ollama").exists() {
        model_source = Some("/media/iso/.ollama");
    }

    if let Some(src) = model_source {
        if let Err(e) = std::os::unix::fs::symlink(src, target_ollama) {
            eprintln!("Failed to link .ollama directory from {}: {}", src, e);
        } else {
            println!("Linked .ollama directory from {} to {}", src, target_ollama);
        }
    } else {
        println!("No offline models found on ISO. Creating empty models folder.");
        let _ = fs::create_dir_all("/var/lib/ollama/.ollama/models");
    }

    // Set correct permissions on ollama and aura directories
    let _ = fs::create_dir_all("/var/lib/aura/macros");
    let _ = Command::new("chown").args(&["-R", "1000:1000", "/var/lib/ollama", "/var/lib/aura"]).status();

    // 6. Start Ollama in background
    println!("Starting Ollama...");
    std::env::set_var("OLLAMA_MODELS", "/var/lib/ollama/.ollama/models");
    if let Err(e) = Command::new("/usr/bin/ollama").arg("serve").spawn() {
        eprintln!("Failed to start ollama: {}", e);
    }

    // 7. Start dbus
    println!("Starting D-Bus daemon...");
    let _ = fs::create_dir_all("/var/run/dbus");
    let _ = Command::new("dbus-uuidgen").arg("--ensure").status();
    let _ = Command::new("dbus-daemon").args(&["--system", "--fork"]).status();

    // 8. Start aura-agent
    println!("Starting aura-agent...");
    if let Err(e) = Command::new("/usr/bin/aura-agent").spawn() {
        eprintln!("Failed to start aura-agent: {}", e);
    }

    // 9. Start openclaw
    if Path::new("/opt/openclaw").exists() {
        println!("Starting openclaw...");
        let _ = Command::new("node")
            .arg("openclaw.mjs")
            .arg("gateway")
            .current_dir("/opt/openclaw")
            .env("NODE_ENV", "production")
            .env("HOME", "/home/aura")
            .spawn();
    }

    // 10. Start graphical server Xorg and Openbox
    println!("Starting Xorg server...");
    let _xorg = Command::new("Xorg")
        .args(&["-nolisten", "tcp", ":0", "vt1"])
        .spawn()
        .expect("Failed to start Xorg");

    // Wait for Xorg socket to appear
    let x_socket = "/tmp/.X11-unix/X0";
    for _ in 0..30 {
        if Path::new(x_socket).exists() {
            println!("Xorg server is ready!");
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }

    std::env::set_var("DISPLAY", ":0");

    // Start Picom compositor for transparency and blur
    println!("Starting Picom compositor...");
    let _picom = Command::new("picom")
        .args(&["--config", "/etc/picom.conf", "-b"])
        .spawn();

    // Start Openbox window manager
    println!("Starting Openbox window manager...");
    let _openbox = Command::new("openbox")
        .spawn()
        .expect("Failed to start openbox");

    // Start Conky system widget
    println!("Starting Conky monitoring widget...");
    let _conky = Command::new("conky")
        .args(&["-c", "/etc/conky.conf", "-d"])
        .spawn();

    // Start aura-gui under user 'aura' (UID 1000) using runuser or directly
    println!("Starting aura-gui...");
    let mut gui = if Path::new("/usr/sbin/runuser").exists() || Path::new("/usr/bin/runuser").exists() {
        Command::new("runuser")
            .args(&["-u", "aura", "--", "/usr/bin/aura-gui"])
            .spawn()
            .expect("Failed to start aura-gui via runuser")
    } else {
        Command::new("/usr/bin/aura-gui")
            .spawn()
            .expect("Failed to start aura-gui directly")
    };

    // 11. Process Reaping (Zombie Reaper loop)
    println!("Init system successfully set up! Entering zombie reaper loop...");
    loop {
        let mut status = 0;
        let pid = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
        if pid > 0 {
            println!("Reaped child process PID: {}", pid);
            
            // Restart GUI if it crashed/exited
            if pid == gui.id() as libc::pid_t {
                println!("WARNING: aura-gui exited. Restarting...");
                thread::sleep(Duration::from_secs(1));
                gui = if Path::new("/usr/sbin/runuser").exists() || Path::new("/usr/bin/runuser").exists() {
                    Command::new("runuser")
                        .args(&["-u", "aura", "--", "/usr/bin/aura-gui"])
                        .spawn()
                        .expect("Failed to restart aura-gui via runuser")
                } else {
                    Command::new("/usr/bin/aura-gui")
                        .spawn()
                        .expect("Failed to restart aura-gui directly")
                };
            }
        }
        thread::sleep(Duration::from_secs(2));
    }
}

fn mount_fs(source: &str, target: &str, fstype: &str, flags: libc::c_ulong, data: Option<&str>) -> std::io::Result<()> {
    use std::ffi::CString;
    let c_source = CString::new(source)?;
    let c_target = CString::new(target)?;
    let c_fstype = CString::new(fstype)?;
    let c_data = match data {
        Some(s) => Some(CString::new(s)?),
        None => None,
    };
    let data_ptr = match &c_data {
        Some(cs) => cs.as_ptr() as *const std::ffi::c_void,
        None => std::ptr::null(),
    };
    let ret = unsafe {
        libc::mount(
            c_source.as_ptr(),
            c_target.as_ptr(),
            c_fstype.as_ptr(),
            flags,
            data_ptr,
        )
    };
    if ret == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}
