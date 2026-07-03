mod kallsyms;
mod memory;
mod stack;

pub const MEMDUMP_DEFAULT_LEN: usize = 128;
pub const MEMDUMP_MAX_LEN: usize = 512;
pub const DEFAULT_DUMP_WORDS: usize = 10;
pub const MAX_DUMP_WORDS: usize = 64;
pub const DEFAULT_TRACE_FRAMES: usize = 10;
pub const MAX_TRACE_FRAMES: usize = 32;

pub(crate) use kallsyms::lookup;
pub(crate) use memory::{
    debug_page_entry, dump_virtual_memory, print_memdebug, print_memstat, run_memtest,
};
pub(crate) use stack::dump_stack_with_options;

#[derive(Clone, Copy)]
pub struct DumpStackOptions {
    pub words: usize,
    pub frames: usize,
    pub print_stack_values: bool,
    pub walk_frames: bool,
    pub scan_stack: bool,
}
