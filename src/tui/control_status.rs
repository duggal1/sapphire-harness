use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct ControlStatusSnapshot {
    pub supervisor: Option<LiveSessionStatus>,
    pub standby_supervisor: Option<LiveSessionStatus>,
    pub workers: HashMap<String, LiveSessionStatus>,
}

#[derive(Debug, Clone)]
pub struct LiveSessionStatus {
    pub name: String,
    pub state: String,
    pub summary: String,
}

pub fn parse_control_status(text: &str) -> ControlStatusSnapshot {
    let mut snapshot = ControlStatusSnapshot::default();
    let mut in_workers = false;

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        if line == "Workers" {
            in_workers = true;
            continue;
        }
        if !line.starts_with("- ") && !line.starts_with("Supervisor: ") && !line.starts_with("Standby Supervisor: ") {
            in_workers = false;
        }

        if let Some(session) = parse_named_session_line(line.strip_prefix("Supervisor: ")) {
            snapshot.supervisor = Some(session);
            continue;
        }
        if let Some(session) = parse_named_session_line(line.strip_prefix("Standby Supervisor: ")) {
            snapshot.standby_supervisor = Some(session);
            continue;
        }
        if in_workers
            && let Some(session) = parse_worker_line(line)
        {
            snapshot.workers.insert(session.name.clone(), session);
        }
    }

    snapshot
}

fn parse_worker_line(line: &str) -> Option<LiveSessionStatus> {
    parse_named_session_line(line.strip_prefix("- "))
}

fn parse_named_session_line(line: Option<&str>) -> Option<LiveSessionStatus> {
    let line = line?.trim();
    let open = line.find('[')?;
    let close = line[open..].find(']')? + open;
    let name = line[..open].trim();
    let state = line[open + 1..close].trim();
    let summary = line[close + 1..].trim();
    if name.is_empty() || state.is_empty() {
        return None;
    }
    Some(LiveSessionStatus {
        name: name.to_owned(),
        state: state.to_owned(),
        summary: summary.to_owned(),
    })
}
