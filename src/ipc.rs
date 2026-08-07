use crate::{
    error::{KResult, KernelError},
    pr_notice, pr_warn,
    locks::Spinlock,
};

pub const SOCKET_BUFFER_SIZE: usize = 1024;
pub const MAX_SOCKETS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum SocketState {
    Unbound,
    Bound,
    Listening,
    Connected,
    Closed,
}

pub struct Socket {
    pub state: SocketState,
    pub rx_buffer: [u8; SOCKET_BUFFER_SIZE],
    pub rx_head: usize,
    pub rx_tail: usize,
    pub peer_index: Option<usize>,
    pub ref_count: usize,
}

impl Socket {
    pub const fn new() -> Self {
        Self {
            state: SocketState::Unbound,
            rx_buffer: [0; SOCKET_BUFFER_SIZE],
            rx_head: 0,
            rx_tail: 0,
            peer_index: None,
            ref_count: 0,
        }
    }
}

const INIT_SOCKET: Spinlock<Socket> = Spinlock::new(Socket::new());
pub static SOCKETS: [Spinlock<Socket>; MAX_SOCKETS] = [INIT_SOCKET; MAX_SOCKETS];

pub fn create_socket() -> KResult<usize> {
    for i in 0..MAX_SOCKETS {
        let mut sock = SOCKETS[i].lock();
        if sock.ref_count == 0 {
            sock.ref_count = 1;
            sock.state = SocketState::Connected;
            sock.rx_head = 0;
            sock.rx_tail = 0;
            sock.peer_index = Some(i);
            return Ok(i);
        }
    }
    pr_warn!("create_socket: no available sockets\n");
    Err(KernelError::ENOBUFS)
}

pub fn read_socket(index: usize, buffer: &mut [u8]) -> KResult<usize> {
    if index >= MAX_SOCKETS {
        pr_warn!("read_socket: invalid socket index {}\n", index);
        return Err(KernelError::EINVAL);
    }

    let mut sock = SOCKETS[index].lock();
    let mut bytes_read = 0;

    // Read from OUR receive buffer
    while bytes_read < buffer.len() && sock.rx_tail != sock.rx_head {
        buffer[bytes_read] = sock.rx_buffer[sock.rx_tail];
        sock.rx_tail = (sock.rx_tail + 1) % SOCKET_BUFFER_SIZE;
        bytes_read += 1;
    }

    Ok(bytes_read)
}

pub fn write_socket(index: usize, buffer: &[u8]) -> KResult<usize> {
    pr_notice!(
        "write_socket: writing {} bytes to socket {}\n",
        buffer.len(),
        index
    );
    if index >= MAX_SOCKETS {
        pr_warn!("write_socket: invalid socket index {}\n", index);
        return Err(KernelError::EINVAL);
    }

    let peer_idx = {
        let sock = SOCKETS[index].lock();
        sock.peer_index
    };

    let target_idx = match peer_idx {
        Some(idx) => idx,
        None => {
            pr_warn!("write_socket: socket {} is not connected\n", index);
            return Err(KernelError::ENOTCONN); // Cannot write to a disconnected socket
        }
    };

    let mut peer_sock = SOCKETS[target_idx].lock();
    let mut bytes_written = 0;

    while bytes_written < buffer.len() {
        let next_head = (peer_sock.rx_head + 1) % SOCKET_BUFFER_SIZE;

        if next_head == peer_sock.rx_tail {
            break; // Peer buffer is full
        }

        let current_head = peer_sock.rx_head;
        peer_sock.rx_buffer[current_head] = buffer[bytes_written];
        peer_sock.rx_head = next_head;
        bytes_written += 1;
    }

    Ok(bytes_written)
}

pub fn close_socket(index: usize) {
    if index >= MAX_SOCKETS {
        pr_warn!("close_socket: invalid socket index {}\n", index);
        return;
    }

    let peer_to_notify;
    {
        let mut sock = SOCKETS[index].lock();
        if sock.ref_count > 0 {
            sock.ref_count -= 1;
        }
        if sock.ref_count == 0 {
            sock.state = SocketState::Closed;
        }
        peer_to_notify = sock.peer_index;
    }

    // If we sever the connection, alert the peer (TCP FIN equivalent)
    if let Some(p_idx) = peer_to_notify {
        let mut peer = SOCKETS[p_idx].lock();
        peer.peer_index = None;
        peer.state = SocketState::Closed;
    }
}
