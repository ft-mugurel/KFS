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
    /// Returns false if queue is full. A task is only queued once.
    pub(crate) unsafe fn sleep(&mut self) -> bool {
        let task = unsafe { sched::current().as_mut() }.unwrap();
        let pid = task.pid;
        if !self.waiters[..self.count].contains(&pid) {
            if self.count >= MAX_WAITERS {
                return false;
            }
            self.waiters[self.count] = pid;
            self.count += 1;
        }

        sched::set_state(task, ProcessState::Waiting);
        true
    }

    /// Wakes the first queued task that is still Waiting, skipping stale entries.
    pub(crate) unsafe fn wake_one(&mut self) {
        let mut table = PROCESS_TABLE.lock();
        while self.count > 0 {
            let pid = self.waiters[0] as usize;
            // Shift remaining waiters
            for i in 0..self.count - 1 {
                self.waiters[i] = self.waiters[i + 1];
            }
            self.count -= 1;

            if let Some(ref mut task) = table[pid]
                && task.state == ProcessState::Waiting
            {
                sched::set_state(task, ProcessState::Ready);
                return;
            }
        }
    }

    pub(crate) unsafe fn wake_all(&mut self) {
        let mut table = PROCESS_TABLE.lock();
        for p_pid in self.waiters.iter().take(self.count) {
            let pid = *p_pid as usize;
            if let Some(ref mut task) = table[pid]
                && task.state == ProcessState::Waiting
            {
                sched::set_state(task, ProcessState::Ready);
            }
        }
        self.count = 0;
    }
}
