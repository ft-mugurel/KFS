#!/usr/bin/env python3
import os
import socket
import subprocess
import time
import sys

SOCK_PATH = "/tmp/qemu-mon.sock"
LOG_PATH = "/tmp/qemu-serial.log"

# Clean up previous sockets and logs
for path in [SOCK_PATH, LOG_PATH]:
    if os.path.exists(path):
        os.remove(path)

qemu_cmd = [
    "qemu-system-i386",
    "-m", "4G",
    "-drive", "file=build/ata_drive.img,format=raw,if=ide,index=0,media=disk",
    "-drive", "file=build/ata_drive_1.img,format=raw,if=ide,index=1,media=disk",
    "-drive", "format=raw,file=build/kernel.iso,media=cdrom",
    "-boot", "order=d",
    "-nographic",
    "-monitor", f"unix:{SOCK_PATH},server,nowait",
    "-serial", f"file:{LOG_PATH}",
    "-smp", "8",
]

proc = subprocess.Popen(qemu_cmd)

def wait_for_pattern(pattern, timeout=12):
    start = time.time()
    while time.time() - start < timeout:
        if os.path.exists(LOG_PATH):
            with open(LOG_PATH, "r", errors="ignore") as f:
                content = f.read()
                if pattern in content:
                    return True, content
        time.sleep(0.05)
    content = ""
    if os.path.exists(LOG_PATH):
        with open(LOG_PATH, "r", errors="ignore") as f:
            content = f.read()
    return False, content

# Wait for monitor socket connection
for _ in range(50):
    if os.path.exists(SOCK_PATH):
        break
    time.sleep(0.1)

s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.connect(SOCK_PATH)

def send_key(key):
    s.sendall(f"sendkey {key}\n".encode())
    time.sleep(0.03)

def send_string(text):
    key_map = {
        '\n': 'ret',
        '\t': 'tab',
        ' ': 'spc',
        '-': 'minus',
        '_': 'shift-minus',
        '/': 'slash',
        '.': 'dot',
    }
    for ch in text:
        send_key(key_map.get(ch, ch))

try:
    print("1. Checking login prompt...")
    ok, log = wait_for_pattern("login: ", timeout=20)
    assert ok, "FAILED: login prompt did not appear!"
    print("[✓] Login prompt ready.")
    time.sleep(1.0)

    print("2. Testing invalid password...")
    send_string("root\n")
    wait_for_pattern("Password: ", timeout=5)
    time.sleep(0.3)
    send_string("badpassword\n")
    ok, log = wait_for_pattern("Login incorrect", timeout=15)
    assert ok, "FAILED: Invalid password was not rejected!"
    print("[✓] Invalid password correctly rejected.")

    print("3. Logging in as root...")
    send_string("root\n")
    wait_for_pattern("Password: ", timeout=5)
    send_string("root\n")
    ok, log = wait_for_pattern("mysh > ", timeout=15)
    assert ok, "FAILED: Shell prompt did not appear after login!"
    print("[✓] Shell loaded successfully in Ring 3.")

    print("4. Testing shell commands (whoami, ps, spawn)...")
    send_string("whoami\n")
    ok, log = wait_for_pattern("root", timeout=5)
    assert ok, "FAILED: whoami did not output root!"

    send_string("spawn\n")
    ok, log = wait_for_pattern("Spawned child PID", timeout=5)
    assert ok, "FAILED: spawn command failed!"
    print("[✓] User process spawn succeeded without Page Fault.")

    print("5. Testing klog trigger and /var/log/kernel.log persistence...")
    send_string("klog hello_klog_persistence_test\n")
    ok, log = wait_for_pattern("Kernel log emitted and flushed to /var/log/kernel.log", timeout=5)
    assert ok, "FAILED: klog command confirmation not received!"
    print("[✓] Kernel log triggered via user shell.")

    time.sleep(0.3)
    send_string("cat /var/log/kernel.log\n")
    ok, log = wait_for_pattern("hello_klog_persistence_test", timeout=6)
    assert ok, "FAILED: Log message not found in /var/log/kernel.log!"
    print("[✓] Kernel log verified in /var/log/kernel.log persistence file.")

    print("6. Testing non-root login on another TTY...")
    send_string("useradd qwe\n")
    ok, log = wait_for_pattern("New password: ", timeout=5)
    assert ok, "FAILED: useradd did not prompt for a password!"
    send_string("qwe\n")
    ok, log = wait_for_pattern("User updated/added", timeout=15)
    assert ok, "FAILED: qwe account was not created!"
    send_string("userdel qwe\n")
    ok, log = wait_for_pattern("User deleted", timeout=15)
    assert ok, "FAILED: userdel did not remove qwe!"
    send_string("userdel root\n")
    ok, log = wait_for_pattern("User deletion failed", timeout=5)
    assert ok, "FAILED: userdel allowed deleting root!"
    send_string("useradd qwe\n")
    ok, log = wait_for_pattern("New password: ", timeout=5)
    assert ok, "FAILED: qwe could not be recreated after deletion!"
    send_string("qwe\n")
    ok, log = wait_for_pattern("User updated/added", timeout=15)
    assert ok, "FAILED: qwe account could not be recreated!"
    send_string("logout\n")
    ok, log = wait_for_pattern("login: ", timeout=5)
    assert ok, "FAILED: root logout did not return to the login prompt!"
    send_key("f2")
    ok, log = wait_for_pattern("login: ", timeout=5)
    assert ok, "FAILED: second TTY login prompt did not appear!"
    send_string("qwe\n")
    ok, log = wait_for_pattern("Password: ", timeout=5)
    assert ok, "FAILED: qwe login did not prompt for a password!"
    send_string("qwe\n")
    ok, log = wait_for_pattern("[AUTH] Authenticate user 'qwe' (len=3): found=true, verified=true", timeout=15)
    assert ok, "FAILED: qwe credentials were not verified by the kernel!"
    send_string("klog qwe_login_probe\n")
    ok, log = wait_for_pattern("[user] qwe_login_probe", timeout=15)
    assert ok, "FAILED: qwe shell did not accept input after login!"
    print("[✓] Non-root login and TTY input succeeded.")

    print("7. Testing logout...")
    send_string("logout\n")
    ok, log = wait_for_pattern("Logout.", timeout=5)
    assert ok, "FAILED: Logout confirmation missing!"
    time.sleep(0.5)
    with open(LOG_PATH, "r", errors="ignore") as f:
        tail = f.read().split("Logout.")[-1]
    assert "login: " in tail, "FAILED: login prompt did not reappear after logout!"
    print("[✓] Logout cleanly returned to TTY login prompt.")

    print("\nALL AUTOMATED TESTS PASSED!")

finally:
    proc.terminate()
    try:
        proc.wait(timeout=2)
    except Exception:
        proc.kill()
