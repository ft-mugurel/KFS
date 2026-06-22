#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileDescriptor {
	TTY(usize),
	Socket(usize),
	// File(usize), // in kfs-6
}