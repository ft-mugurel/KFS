use super::ContextFrame;
use crate::error::KernelError;

impl ContextFrame {
    pub const fn arg1(&self) -> u32 {
        self.ebx
    }
    pub const fn arg2(&self) -> u32 {
        self.ecx
    }
    pub const fn arg3(&self) -> u32 {
        self.edx
    }
    pub const fn arg4(&self) -> u32 {
        self.esi
    }
    pub const fn arg5(&self) -> u32 {
        self.edi
    }
    pub const fn arg6(&self) -> u32 {
        self.ebp
    }
    pub fn set_return_value(&mut self, value: u32) {
        self.eax = value;
    }
    pub fn set_return_error(&mut self, error: KernelError) {
        self.eax = !(error as u32);
    }
}
