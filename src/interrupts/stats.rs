use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::smp::MAX_CPUS;

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub(crate) enum IrqSource {
    Pit,
    Keyboard,
    LapicTimer,
    ShutdownIpi,
}

impl IrqSource {
    pub(crate) const ALL: [IrqSource; 4] = [
        IrqSource::Pit,
        IrqSource::Keyboard,
        IrqSource::LapicTimer,
        IrqSource::ShutdownIpi,
    ];

    pub(crate) fn label(self) -> IrqLabel {
        use crate::startup_config::pic;
        match self {
            IrqSource::Pit => IrqLabel::Line(0),
            IrqSource::Keyboard => IrqLabel::Line(pic::KEYBOARD_IRQ_LINE),
            IrqSource::LapicTimer => IrqLabel::Tag("LOC"),
            IrqSource::ShutdownIpi => IrqLabel::Tag("SHT"),
        }
    }

    pub(crate) fn description(self) -> &'static str {
        match self {
            IrqSource::Pit => "XT-PIC  timer",
            IrqSource::Keyboard => "XT-PIC  i8042",
            IrqSource::LapicTimer => "Local timer interrupts",
            IrqSource::ShutdownIpi => "Shutdown IPIs",
        }
    }
}

pub(crate) enum IrqLabel {
    Line(u8),
    Tag(&'static str),
}

const SOURCES: usize = IrqSource::ALL.len();

static COUNTS: [[AtomicU32; MAX_CPUS]; SOURCES] =
    [const { [const { AtomicU32::new(0) }; MAX_CPUS] }; SOURCES];
static INSTALLED: [AtomicBool; SOURCES] = [const { AtomicBool::new(false) }; SOURCES];

pub(crate) fn mark_installed(source: IrqSource) {
    INSTALLED[source as usize].store(true, Ordering::Relaxed);
}

pub(crate) fn is_installed(source: IrqSource) -> bool {
    INSTALLED[source as usize].load(Ordering::Relaxed)
}

pub(crate) fn count(source: IrqSource) {
    let cpu_id = unsafe { crate::sched::current_cpu() } as usize;
    if let Some(counter) = COUNTS[source as usize].get(cpu_id) {
        counter.fetch_add(1, Ordering::Relaxed);
    }
}

pub(crate) fn count_on(source: IrqSource, cpu_id: usize) -> u32 {
    COUNTS[source as usize]
        .get(cpu_id)
        .map_or(0, |counter| counter.load(Ordering::Relaxed))
}

pub(crate) fn total() -> u32 {
    COUNTS
        .iter()
        .flatten()
        .fold(0u32, |sum, counter| sum.wrapping_add(counter.load(Ordering::Relaxed)))
}
