use crate::startup_config::pic;
use crate::x86::{io_wait, outb};

pub(crate) fn init_pic() {
    outb(pic::MASTER_COMMAND_PORT, pic::ICW1_INIT);
    io_wait();
    outb(pic::MASTER_DATA_PORT, pic::MASTER_IRQ_OFFSET);
    io_wait();
    outb(pic::MASTER_DATA_PORT, pic::SLAVE_CONNECTED_TO_IRQ);
    io_wait();
    outb(pic::MASTER_DATA_PORT, pic::ICW4_8086);
    io_wait();

    outb(pic::SLAVE_COMMAND_PORT, pic::ICW1_INIT);
    io_wait();
    outb(pic::SLAVE_DATA_PORT, pic::SLAVE_IRQ_OFFSET);
    io_wait();
    outb(pic::SLAVE_DATA_PORT, pic::CASCADE_IDENTITY);
    io_wait();
    outb(pic::SLAVE_DATA_PORT, pic::ICW4_8086);
    io_wait();

    outb(pic::MASTER_DATA_PORT, pic::MASK_ALL);
    io_wait();
    outb(pic::SLAVE_DATA_PORT, pic::MASK_ALL);
    io_wait();

    outb(pic::MASTER_DATA_PORT, pic::MASK_ENABLE_TIMER_KEYBOARD);
    io_wait();
    outb(pic::SLAVE_DATA_PORT, pic::MASK_ALL);
    io_wait();
}
