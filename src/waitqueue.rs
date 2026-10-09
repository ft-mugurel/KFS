use crate::sched::{self, PROCESS_TABLE, ProcessState};

pub(crate) const MAX_WAITERS: usize = 16;

pub(crate) struct WaitQueue {
    waiters: [u32; MAX_WAITERS], // PIDs
    count: usize,
}

impl WaitQueue {
    pub(crate) const fn new() -> Self {
        Self { waiters: [0; MAX_WAITERS], count: 0 }
    }

    /// Register current task as waiting. Sets task to Waiting state.
    /// Returns false if queue is full.
    pub(crate) unsafe fn sleep(&mut self) -> bool {
        if self.count >= MAX_WAITERS {
            return false;
        }
        let pid = sched::current_pid();
        self.waiters[self.count] = pid;
        self.count += 1;

        let task = sched::current().as_mut().unwrap();
        sched::set_state(task, ProcessState::Waiting);
        true
    }

    pub(crate) unsafe fn wake_one(&mut self) {
        if self.count == 0 {
            return;
        }
        let pid = self.waiters[0] as usize;
        // Shift remaining waiters
        for i in 0..self.count - 1 {
            self.waiters[i] = self.waiters[i + 1];
        }
        self.count -= 1;

        let mut table = PROCESS_TABLE.lock();
        if let Some(ref mut task) = table[pid] {
            if task.state == ProcessState::Waiting {
                sched::set_state(task, ProcessState::Ready);
            }
        }
    }

    pub(crate) unsafe fn wake_all(&mut self) {
        let mut table = PROCESS_TABLE.lock();
        for i in 0..self.count {
            let pid = self.waiters[i] as usize;
            if let Some(ref mut task) = table[pid] {
                if task.state == ProcessState::Waiting {
                    sched::set_state(task, ProcessState::Ready);
                }
            }
        }
        self.count = 0;
    }

    pub(crate) fn remove(&mut self, pid: u32) {
        let mut i = 0;
        while i < self.count {
            if self.waiters[i] == pid {
                for j in i..self.count - 1 {
                    self.waiters[j] = self.waiters[j + 1];
                }
                self.count -= 1;
            } else {
                i += 1;
            }
        }
    }
}
