use crate::cli::LaunchConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MissionProfile {
    /// Whether the mission requires multi-agent coordination (affects prompt content).
    pub coordination_focused: bool,
    /// Always true — full safety nets are always enabled.
    pub enable_repair_supervisor: bool,
    /// Always true — supervisor health is always monitored.
    pub enable_supervisor_health_recovery: bool,
    /// Always true — state cards always sent to supervisor.
    pub enable_state_cards: bool,
    /// Always true — protocol reminders always sent.
    pub enable_protocol_reminders: bool,
    /// Always true — health probes always active before stall.
    pub enable_health_probes: bool,
}

impl MissionProfile {
    pub fn from_launch(config: &LaunchConfig) -> Self {
        let lowered = config.mission.to_ascii_lowercase();
        let coordination_focused = contains_any(
            &lowered,
            &[
                "teammate",
                "coordinate",
                "coordination",
                "work together",
                "collaborate",
                "talk to your teammate",
                "prove",
                "mail",
                "agent-to-agent",
                "inter-agent",
            ],
        );

        // ALL safety nets always enabled. No "lean" mode. No cheaping out.
        Self {
            coordination_focused,
            enable_repair_supervisor: true,
            enable_supervisor_health_recovery: true,
            enable_state_cards: true,
            enable_protocol_reminders: true,
            enable_health_probes: true,
        }
    }
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}
