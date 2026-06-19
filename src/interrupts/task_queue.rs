use crate::spin::Spinlock;

const QUEUE_SIZE: usize = 128;

pub type Task = fn();

struct TaskQueue {
    tasks: [Option<Task>; QUEUE_SIZE],
    head: usize,
    tail: usize,
}

impl TaskQueue {
    const fn new() -> Self {
        Self { tasks: [None; QUEUE_SIZE], head: 0, tail: 0 }
    }

    fn push(&mut self, task: Task) -> bool {
        let next_head = (self.head + 1) % QUEUE_SIZE;
        if next_head == self.tail {
            return false;
        }
        self.tasks[self.head] = Some(task);
        self.head = next_head;
        true
    }

    fn pop(&mut self) -> Option<Task> {
        if self.head == self.tail {
            return None; // Queue is empty
        }
        let task = self.tasks[self.tail].take();
        self.tail = (self.tail + 1) % QUEUE_SIZE;
        task
    }
}

static TASK_QUEUE: Spinlock<TaskQueue> = Spinlock::new(TaskQueue::new());

pub fn schedule_task(task: Task) {
    let mut queue = TASK_QUEUE.lock();
    if !queue.push(task) {
        crate::pr_warn!("Task queue overflow! Dropped task.\n");
    }
}

pub fn execute_tasks() {
    loop {
        let task_opt = {
            let mut queue = TASK_QUEUE.lock();
            queue.pop()
        };

        if let Some(task) = task_opt {
            task();
        } else {
            break;
        }
    }
}
