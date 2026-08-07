use crate::pr_info;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum InitcallLevel {
    Early,
    Arch,
    Core,
    Subsys,
    Late,
}

impl InitcallLevel {
    fn as_str(self) -> &'static str {
        match self {
            InitcallLevel::Early => "early",
            InitcallLevel::Arch => "arch",
            InitcallLevel::Core => "core",
            InitcallLevel::Subsys => "subsys",
            InitcallLevel::Late => "late",
        }
    }
}

pub type Initcall = fn();

#[derive(Clone, Copy)]
pub struct InitcallStep {
    pub name: &'static str,
    pub func: Initcall,
}

pub fn run_initcalls(level: InitcallLevel, initcalls: &[InitcallStep]) {
    for initcall in initcalls {
        pr_info!("[initcall:{}] {}\n", level.as_str(), initcall.name);
        (initcall.func)();
    }
}