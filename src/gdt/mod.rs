mod gdt;

pub(crate) const KERNEL_CODE_SEL: u16 = (1 << 3) | 0;
pub(crate) const KERNEL_DATA_SEL: u16 = (2 << 3) | 0;
pub(crate) const USER_CODE_SEL: u16 = (4 << 3) | 3;
pub(crate) const USER_DATA_SEL: u16 = (5 << 3) | 3;
pub(crate) const TSS_SEL: u16 = (7 << 3) | 0;

pub(crate) use gdt::{load_gdt, set_kernel_stack, TSS};
