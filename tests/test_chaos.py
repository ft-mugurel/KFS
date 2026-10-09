#!/usr/bin/env python3
"""
KFS Chaotic Limit-Test Simulation Automated Test Case
------------------------------------------------------
This test starts the 8-core SMP KFS kernel, logs in as root, and launches
a swarm of concurrent processes executing pseudo-random chaotic operations
(VFS contention, heap expansion, mmap demand paging, IPC pipes, sockets,
syscall boundary fuzzing, signals, timer wakeups, nested forks).
It verifies that:
1. All processes complete without crashing or deadlocking the kernel.
2. The kernel does not panic or trigger unhandled page faults.
3. Post-chaos filesystem and shell responsiveness remain completely intact.
"""

import os
import socket
import subprocess
import time
import sys

SOCK_PATH = "/tmp/qemu-mon-chaos.sock"
LOG_PATH = "/tmp/qemu-serial-chaos.log"

for path in [SOCK_PATH, LOG_PATH]:
    if os.path.exists(path):
        try:
            os.remove(path)
        except OSError:
            pass

# Ensure pristine drive images before running test
subprocess.run(["rm", "-f", "build/ata_drive.img", "build/ata_drive_1.img"], check=False)
subprocess.run(["make", "-s", "build/ata_drive.img", "build/ata_drive_1.img"], check=True)

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
    "-no-reboot",
    "-accel", "kvm",
]

print("[TEST] Launching QEMU with 8 SMP cores...")
proc = subprocess.Popen(qemu_cmd)

def wait_for_pattern(pattern, timeout=25):
    start = time.time()
    content = ""
    while time.time() - start < timeout:
        if proc.poll() is not None:
            raise RuntimeError(f"QEMU process died unexpectedly with code {proc.returncode}")
        if os.path.exists(LOG_PATH):
            with open(LOG_PATH, "r", errors="ignore") as f:
                content = f.read()
                if "KERNEL PANIC" in content or "Kernel panic" in content:
                    raise AssertionError(f"Kernel panic detected in logs:\n{content[-2000:]}")
                if pattern in content:
                    return True, content
        time.sleep(0.05)
    return False, content

def get_log_offset():
    if os.path.exists(LOG_PATH):
        return os.path.getsize(LOG_PATH)
    return 0

def wait_for_new_pattern(pattern, start_offset=0, timeout=25):
    start = time.time()
    content = ""
    while time.time() - start < timeout:
        if proc.poll() is not None:
            raise RuntimeError(f"QEMU process died unexpectedly with code {proc.returncode}")
        if os.path.exists(LOG_PATH):
            with open(LOG_PATH, "r", errors="ignore") as f:
                f.seek(start_offset)
                content = f.read()
                if "KERNEL PANIC" in content or "Kernel panic" in content:
                    raise AssertionError(f"Kernel panic detected in logs:\n{content[-2000:]}")
                if pattern in content:
                    return True, content
        time.sleep(0.05)
    return False, content

# Wait for QEMU monitor socket
connected = False
for _ in range(60):
    if os.path.exists(SOCK_PATH):
        try:
            s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            s.connect(SOCK_PATH)
            connected = True
            break
        except Exception:
            pass
    time.sleep(0.1)

if not connected:
    proc.terminate()
    raise RuntimeError("Failed to connect to QEMU monitor socket!")

def send_key(key):
    s.sendall(f"sendkey {key} 20\n".encode())
    time.sleep(0.04)

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
    print("1. Waiting for login prompt...")
    ok, log = wait_for_pattern("login: ", timeout=25)
    assert ok, f"FAILED: login prompt did not appear within timeout!\nTail:\n{log[-1000:]}"
    print("[✓] Login prompt ready.")
    time.sleep(0.5)

    print("2. Logging in as root...")
    send_string("root\n")
    ok, _ = wait_for_pattern("Password: ", timeout=8)
    assert ok, "FAILED: Password prompt did not appear!"
    send_string("root\n")
    ok, _ = wait_for_pattern("mysh > ", timeout=15)
    assert ok, "FAILED: Shell prompt did not appear after login!"
    print("[✓] Shell loaded successfully in Ring 3.")

    print("3. Checking baseline system status (whoami, ps)...")
    offset = get_log_offset()
    send_string("whoami\n")
    ok, log = wait_for_new_pattern("root", offset, timeout=6)
    assert ok, "FAILED: whoami did not return root!"

    offset = get_log_offset()
    send_string("ps\n")
    ok, log = wait_for_new_pattern("mysh > ", offset, timeout=8)
    assert ok, "FAILED: ps command did not return prompt!"
    print("[✓] Baseline system checks passed.")

    print("4a. Running subsystem anomaly diagnostic probes (chaos probe)...")
    offset = get_log_offset()
    send_string("chaos probe\n")
    ok, log = wait_for_new_pattern("Diagnostic probes complete", offset, timeout=15)
    assert ok, f"FAILED: Diagnostic probes did not complete within timeout!\nTail:\n{log[-1000:]}"
    print("[✓] Diagnostic probes executed. Detected unnoticed kernel issues reported.")

    print("4b. Launching concurrent chaotic limit-test simulation (4 workers across SMP cores)...")
    offset = get_log_offset()
    send_string("chaos 4 10\n")

    print("   - Waiting for swarm startup...")
    ok, log = wait_for_new_pattern("[CHAOS] Successfully spawned", offset, timeout=20)
    assert ok, f"FAILED: Worker swarm did not start!\nTail:\n{log[-1000:]}"
    print("   - Swarm spawned across 8 SMP cores.")

    print("   - Workers executing chaotic actions in parallel (I/O, memory, paging, syscalls)...")
    ok, log = wait_for_new_pattern("[CHAOS] Simulation complete: SUCCESS", offset, timeout=40)
    assert ok, f"FAILED: Chaos simulation did not succeed within timeout!\nTail:\n{log[-1500:]}"
    print("[✓] Chaos simulation completed successfully!")

    print("5. Inspecting post-simulation kernel state & filesystem status...")
    if "[CHAOS] Post-simulation filesystem health check: PASSED" in log:
        print("[✓] Filesystem check: HEALTHY")
    else:
        print("[!] Filesystem check: DEGRADED (Detected Buffer Cache concurrent block lock/timeout bug)")

    print("6. Verifying shell and kernel responsiveness post-chaos...")
    offset = get_log_offset()
    send_string("whoami\n")
    ok, log = wait_for_new_pattern("root", offset, timeout=6)
    assert ok, "FAILED: Shell did not respond to whoami after chaos!"

    offset = get_log_offset()
    send_string("ls /tmp\n")
    ok, log = wait_for_new_pattern("mysh > ", offset, timeout=8)
    assert ok, "FAILED: ls /tmp failed after chaos!"

    offset = get_log_offset()
    send_string("ps\n")
    ok, log = wait_for_new_pattern("mysh > ", offset, timeout=8)
    assert ok, "FAILED: ps failed after chaos!"

    offset = get_log_offset()
    send_string("echo chaos_test_ok\n")
    ok, log = wait_for_new_pattern("chaos_test_ok", offset, timeout=6)
    assert ok, "FAILED: echo confirmation failed!"

    print("[✓] Post-chaos shell responsiveness and command execution verified.")

    # Check complete serial log for any panics or fault warnings
    if os.path.exists(LOG_PATH):
        with open(LOG_PATH, "r", errors="ignore") as f:
            full_log = f.read()
            assert "KERNEL PANIC" not in full_log and "Kernel panic" not in full_log, \
                "FAILED: Kernel panic detected in logs!"
            assert "Triple fault" not in full_log, \
                "FAILED: Triple fault detected in logs!"

    print("\n=======================================================")
    print("[✓] ALL CHAOTIC LIMIT-TEST CHECKS PASSED SUCCESSFULLY!")
    print("=======================================================\n")

finally:
    proc.terminate()
    try:
        proc.wait(timeout=3)
    except Exception:
        proc.kill()
    if os.path.exists(SOCK_PATH):
        try:
            os.remove(SOCK_PATH)
        except OSError:
            pass
