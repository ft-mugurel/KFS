use crate::{
    error::{KResult, KernelError},
    locks::Spinlock,
    pr_debug, pr_warn,
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
    Disconnected,
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

#[allow(dead_code)]
pub fn create_socket_pair() -> KResult<(usize, usize)> {
    let mut first = None;

    for i in 0..MAX_SOCKETS {
        let mut sock = SOCKETS[i].lock();
        if sock.ref_count == 0 {
            sock.ref_count = 1;
            first = Some(i);
            break;
        }
    }

    let idx1 = match first {
        Some(i) => i,
        None => return Err(KernelError::ENOBUFS),
    };

    let mut second = None;
    for i in 0..MAX_SOCKETS {
        if i == idx1 {
            continue;
        }
        let mut sock = SOCKETS[i].lock();
        if sock.ref_count == 0 {
            sock.ref_count = 1;
            second = Some(i);
            break;
        }
    }

    let idx2 = match second {
        Some(i) => i,
        None => {
            let mut sock1 = SOCKETS[idx1].lock();
            sock1.ref_count = 0;
            sock1.state = SocketState::Closed;
            return Err(KernelError::ENOBUFS);
        }
    };

    {
        let mut sock1 = SOCKETS[idx1].lock();
        sock1.state = SocketState::Connected;
        sock1.rx_head = 0;
        sock1.rx_tail = 0;
        sock1.peer_index = Some(idx2);
    }

    {
        let mut sock2 = SOCKETS[idx2].lock();
        sock2.state = SocketState::Connected;
        sock2.rx_head = 0;
        sock2.rx_tail = 0;
        sock2.peer_index = Some(idx1);
    }

    Ok((idx1, idx2))
}

pub fn read_socket(index: usize, buffer: &mut [u8]) -> KResult<usize> {
    if index >= MAX_SOCKETS {
        pr_warn!("read_socket: invalid socket index {}\n", index);
        return Err(KernelError::EINVAL);
    }

    let mut sock = SOCKETS[index].lock();
    if sock.ref_count == 0 || sock.state == SocketState::Closed {
        pr_warn!("read_socket: socket {} is not open\n", index);
        return Err(KernelError::EBADF);
    }

    if buffer.is_empty() {
        return Ok(0);
    }

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
    pr_debug!(
        "write_socket: writing {} bytes to socket {}\n",
        buffer.len(),
        index
    );
    if index >= MAX_SOCKETS {
        pr_warn!("write_socket: invalid socket index {}\n", index);
        return Err(KernelError::EINVAL);
    }

    if buffer.is_empty() {
        return Ok(0);
    }

    let (peer_idx, is_open) = {
        let sock = SOCKETS[index].lock();
        (
            sock.peer_index,
            sock.ref_count > 0 && sock.state != SocketState::Closed,
        )
    };

    if !is_open {
        pr_warn!("write_socket: socket {} is not open\n", index);
        return Err(KernelError::EBADF);
    }

    let target_idx = match peer_idx {
        Some(idx) => idx,
        None => {
            pr_warn!("write_socket: socket {} is not connected\n", index);
            return Err(KernelError::EPIPE);
        }
    };

    if target_idx >= MAX_SOCKETS {
        return Err(KernelError::EINVAL);
    }

    let mut peer_sock = SOCKETS[target_idx].lock();
    if peer_sock.ref_count == 0 || peer_sock.state == SocketState::Closed {
        pr_warn!("write_socket: peer socket {} is closed\n", target_idx);
        return Err(KernelError::EPIPE);
    }

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

    if bytes_written == 0 {
        return Err(KernelError::ENOBUFS);
    }

    Ok(bytes_written)
}

pub fn close_socket(index: usize) {
    if index >= MAX_SOCKETS {
        pr_warn!("close_socket: invalid socket index {}\n", index);
        return;
    }

    let peer_to_notify = {
        let mut sock = SOCKETS[index].lock();
        if sock.ref_count > 0 {
            sock.ref_count -= 1;
        }
        if sock.ref_count == 0 {
            sock.state = SocketState::Closed;
            sock.rx_head = 0;
            sock.rx_tail = 0;
            let peer = sock.peer_index;
            sock.peer_index = None;
            if peer == Some(index) {
                // Loopback: self-cleared, no external peer to notify
                None
            } else {
                peer
            }
        } else {
            None // Still has active references, do not notify peer
        }
    };

    // If we severed the connection to a separate peer, inform the peer
    if let Some(p_idx) = peer_to_notify {
        if p_idx < MAX_SOCKETS {
            let mut peer = SOCKETS[p_idx].lock();
            peer.peer_index = None;
            if peer.state == SocketState::Connected {
                peer.state = SocketState::Disconnected;
            }
        }
    }
}
