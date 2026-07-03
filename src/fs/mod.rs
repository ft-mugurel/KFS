mod ext2;
mod vfs;

#[derive(Debug, Clone, Copy)]
pub struct OpenFile {
    pub node: *mut VfsNode,
    pub offset: u32,
    pub ref_count: usize,
}

pub const MAX_OPEN_FILES: usize = 256;
pub static mut OPEN_FILE_TABLE: [Option<OpenFile>; MAX_OPEN_FILES] = {
    const EMPTY: Option<OpenFile> = None;
    [EMPTY; MAX_OPEN_FILES]
};

#[repr(C, packed)]
struct Ext2Superblock {
    pub inodes_count: u32,
    pub blocks_count: u32,
    pub r_blocks_count: u32,
    pub free_blocks_count: u32,
    pub free_inodes_count: u32,
    pub first_data_block: u32,
    pub log_block_size: u32,
    pub log_frag_size: u32,
    pub blocks_per_group: u32,
    pub frags_per_group: u32,
    pub inodes_per_group: u32,
    pub mtime: u32,
    pub wtime: u32,
    pub mnt_count: u16,
    pub max_mnt_count: u16,
    pub magic: u16,
    pub state: u16,
    pub errors: u16,
    pub minor_rev_level: u16,
    pub lastcheck: u32,
    pub checkinterval: u32,
    pub creator_os: u32,
    pub rev_level: u32,
    pub def_resuid: u16,
    pub def_resgid: u16,
    pub first_ino: u32,
    pub inode_size: u16,
    pub block_group_nr: u16,
    pub feature_compat: u32,
    pub feature_incompat: u32,
    pub feature_ro_compat: u32,
    pub uuid: [u8; 16],
    pub vol_name: [char; 16],
    pub last_mounted: [char; 64],
    pub algo_bitmap: u32,
    pub prealloc_blocks: u8,
    pub prealloc_dir_blocks: u8,
    pub reserved_gdt_blocks: u16,
}

#[repr(C, packed)]
pub struct Ext2GroupDesc {
    pub bg_block_bitmap: u32,
    pub bg_inode_bitmap: u32,
    pub bg_inode_table: u32, // The physical block where the Inodes live
    pub bg_free_blocks_count: u16,
    pub bg_free_inodes_count: u16,
    pub bg_used_dirs_count: u16,
    pub bg_pad: u16,
    pub bg_reserved: [u32; 3],
}

#[repr(C, packed)]
pub struct Ext2Inode {
    pub i_mode: u16, // File type and permissions
    pub i_uid: u16,
    pub i_size: u32, // Size in bytes
    pub i_atime: u32,
    pub i_ctime: u32,
    pub i_mtime: u32,
    pub i_dtime: u32,
    pub i_gid: u16,
    pub i_links_count: u16,
    pub i_blocks: u32, // Number of 512-byte sectors used
    pub i_flags: u32,
    pub i_osd1: u32,
    pub i_block: [u32; 15], // Pointers to data blocks (Direct, Indirect, etc.)
    pub i_generation: u32,
    pub i_file_acl: u32,
    pub i_dir_acl: u32,
    pub i_faddr: u32,
    pub i_osd2: [u32; 3],
}

#[repr(C, packed)]
pub struct Ext2DirEntry {
    pub inode: u32,
    pub rec_len: u16,
    pub name_len: u8,
    pub file_type: u8,
    pub name: *const u8, // Variable length name
}

// Ext2 Inodes are 1-indexed. Inode 2 is the Root Directory.
pub const EXT2_ROOT_INODE: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VfsNodeType {
    Unknown = 0,
    Fifo = 1,
    CharDevice = 2,
    Directory = 3,
    BlockDevice = 4,
    File = 5,
    Socket = 6,
    Symlink = 7,
}

pub struct VfsNode {
    pub name: [u8; 256],           // Name
    pub size: u32,                 // Size
    pub node_type: VfsNodeType,    // Type
    pub inode: u32,                // Inode
    pub links: u32,                // Links
    pub master: *mut VfsNode,      // Master (Mount point / device connection)
    pub father: *mut VfsNode,      // Father (Parent directory)
    pub children: *mut VfsNode,    // Children (Head of the child linked list)
    pub next_of_kin: *mut VfsNode, // Next of kin (Sibling in the same directory)

    pub rights: u16, // Rights (Permissions mask, e.g., 0755)
}

pub use ext2::{dump_ext2_sb, get_ext2_inode, read_ext2_block, write_ext2_block, EXT2_BLOCK_SIZE};

pub use vfs::{
    alloc_vfs_node, ext2_type_to_vfs, print_vfs_tree, read_file, resolve_path, ROOT_NODE,
};
