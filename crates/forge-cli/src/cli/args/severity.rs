use clap::ValueEnum;

#[derive(Clone, Copy, ValueEnum, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum FailOnSeverity {
    Low,
    Medium,
    High,
    Critical,
}

impl FailOnSeverity {
    pub(crate) fn matches(self, sev: forge_types::Severity) -> bool {
        let sev_ord = match sev {
            forge_types::Severity::Low => FailOnSeverity::Low,
            forge_types::Severity::Medium => FailOnSeverity::Medium,
            forge_types::Severity::High => FailOnSeverity::High,
            forge_types::Severity::Critical => FailOnSeverity::Critical,
        };
        sev_ord >= self
    }
}
