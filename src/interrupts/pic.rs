use crate::startup_config::pic;
use crate::x86::outb;

#[inline(always)]
fn io_wait() {
    outb(0x80, 0);
}

pub(crate) fn init_pic() {
    // Master PIC: Başlangıç komutları
    outb(pic::MASTER_COMMAND_PORT, pic::ICW1_INIT); // ICW1: Başlangıç
    io_wait();
    outb(pic::MASTER_DATA_PORT, pic::MASTER_IRQ_OFFSET); // ICW2: Master PIC için vektör offset'i 0x20 (32)
    io_wait();
    outb(pic::MASTER_DATA_PORT, pic::SLAVE_CONNECTED_TO_IRQ); // ICW3: Slave PIC'in IRQ2'ye bağlandığını belirt
    io_wait();
    outb(pic::MASTER_DATA_PORT, pic::ICW4_8086); // ICW4: 8086 modunu ayarla
    io_wait();

    // Slave PIC: Başlangıç komutları
    outb(pic::SLAVE_COMMAND_PORT, pic::ICW1_INIT); // ICW1: Başlangıç
    io_wait();
    outb(pic::SLAVE_DATA_PORT, pic::SLAVE_IRQ_OFFSET); // ICW2: Slave PIC için vektör offset'i 0x28 (40)
    io_wait();
    outb(pic::SLAVE_DATA_PORT, pic::CASCADE_IDENTITY); // ICW3: Slave PIC'in IRQ2'ye bağlandığını belirt
    io_wait();
    outb(pic::SLAVE_DATA_PORT, pic::ICW4_8086); // ICW4: 8086 modunu ayarla
    io_wait();

    // Tüm interruptları maskeler (engeller) - Başlangıçta maskeyi kaldırıyoruz
    outb(pic::MASTER_DATA_PORT, pic::MASK_ALL); // Master PIC'teki tüm interruptları maskeler
    io_wait();
    outb(pic::SLAVE_DATA_PORT, pic::MASK_ALL); // Slave PIC'teki tüm interruptları maskeler
    io_wait();

    outb(pic::MASTER_DATA_PORT, pic::MASK_ENABLE_TIMER_KEYBOARD);
    io_wait();
    outb(pic::SLAVE_DATA_PORT, pic::MASK_ALL); // Slave PIC'teki interruptları maskele (gerekirse)
    io_wait();
}

