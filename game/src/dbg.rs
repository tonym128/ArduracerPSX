//! Low-overhead zero-allocation TTY debug instrumentation for PSX hardware and emulators.

#[cfg(target_arch = "mips")]
use psx_rt::tty;

#[inline(never)]
pub fn print(s: &str) {
    #[cfg(target_arch = "mips")]
    tty::print(s);
    #[cfg(not(target_arch = "mips"))]
    let _ = s;
}

#[inline(never)]
pub fn println(s: &str) {
    #[cfg(target_arch = "mips")]
    tty::println(s);
    #[cfg(not(target_arch = "mips"))]
    let _ = s;
}

#[inline(never)]
pub fn print_hex(val: u32) {
    #[cfg(target_arch = "mips")]
    tty::print_hex_u32(val);
    #[cfg(not(target_arch = "mips"))]
    let _ = val;
}

#[inline(never)]
pub fn print_dec(mut v: u32) {
    #[cfg(target_arch = "mips")]
    {
        if v == 0 {
            tty::print("0");
            return;
        }
        let mut buf = [0u8; 10];
        let mut i = 0;
        while v > 0 {
            buf[i] = b'0' + (v % 10) as u8;
            v /= 10;
            i += 1;
        }
        while i > 0 {
            i -= 1;
            if let Ok(s) = core::str::from_utf8(&buf[i..i + 1]) {
                tty::print(s);
            }
        }
    }
    #[cfg(not(target_arch = "mips"))]
    let _ = v;
}

#[inline(never)]
pub fn log_step(step: u8, total: u8, desc: &str) {
    print("[BOOT ");
    print_dec(step as u32);
    print("/");
    print_dec(total as u32);
    print("] ");
    println(desc);
}

#[inline(never)]
pub fn log_kv_hex(key: &str, val: u32) {
    print("  ");
    print(key);
    print(": 0x");
    print_hex(val);
    println("");
}

#[inline(never)]
pub fn log_kv_dec(key: &str, val: u32) {
    print("  ");
    print(key);
    print(": ");
    print_dec(val);
    println("");
}

#[inline(never)]
pub fn check_faults() {
    #[cfg(target_arch = "mips")]
    {
        let count = psx_rt::interrupts::fault_count();
        if count > 0 {
            print("[FAULT ALERT] Fault count: ");
            print_dec(count);
            print(" BadVAddr: 0x");
            print_hex(psx_rt::interrupts::fault_badvaddr());
            print(" EPC: 0x");
            print_hex(psx_rt::interrupts::fault_epc());
            print(" Cause: 0x");
            print_hex(psx_rt::interrupts::fault_cause());
            println("");
        }
    }
}

#[inline(never)]
pub fn check_stack() {
    #[cfg(target_arch = "mips")]
    psx_rt::assert_stack_headroom();
}
