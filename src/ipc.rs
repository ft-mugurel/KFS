use crate::spin::Spinlock;

pub const SOCKET_BUFFER_SIZE: usize = 1024;
pub const MAX_SOCKETS: usize = 16;

pub struct Socket {
    pub buffer: [u8; SOCKET_BUFFER_SIZE],
    pub head: usize,
    pub tail: usize,
    pub ref_count: usize,
}

impl Socket {
    pub const fn new() -> Self {
        Self {
            buffer: [0; SOCKET_BUFFER_SIZE],
            head: 0,
            tail: 0,
            ref_count: 0,
        }
    }
}

const INIT_SOCKET: Spinlock<Socket> = Spinlock::new(Socket::new());
pub static SOCKETS: [Spinlock<Socket>; MAX_SOCKETS] = [INIT_SOCKET; MAX_SOCKETS];

pub fn create_socket() -> Option<usize> {
    for i in 0..MAX_SOCKETS {
        let mut sock = SOCKETS[i].lock();
        if sock.ref_count == 0 {
            sock.ref_count = 1;
            sock.head = 0;
            sock.tail = 0;
            return Some(i);
        }
    }
    None
}

pub fn close_socket(index: usize) {
    if index < MAX_SOCKETS {
        let mut sock = SOCKETS[index].lock();
        if sock.ref_count > 0 {
            sock.ref_count -= 1;
        }
    }
}