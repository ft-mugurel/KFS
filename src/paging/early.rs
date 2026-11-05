pub unsafe fn read_phys<T: Copy>(addr: u32) -> T {
    unsafe { (addr as *const T).read_unaligned() }
}
