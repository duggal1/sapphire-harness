#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarTab {
    Workers,
    Problems,
    Watchdog,
    Events,
    Supervisor,
}

impl SidebarTab {
    pub const ALL: [Self; 5] = [
        Self::Workers,
        Self::Problems,
        Self::Watchdog,
        Self::Events,
        Self::Supervisor,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Workers => "Workers",
            Self::Problems => "Problems",
            Self::Watchdog => "Watchdog",
            Self::Events => "Events",
            Self::Supervisor => "Supervisor",
        }
    }

    pub fn hotkey(self) -> &'static str {
        match self {
            Self::Workers => "1",
            Self::Problems => "P",
            Self::Watchdog => "2",
            Self::Events => "3",
            Self::Supervisor => "4",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Workers => Self::Problems,
            Self::Problems => Self::Watchdog,
            Self::Watchdog => Self::Events,
            Self::Events => Self::Supervisor,
            Self::Supervisor => Self::Workers,
        }
    }

    pub fn previous(self) -> Self {
        match self {
            Self::Workers => Self::Supervisor,
            Self::Problems => Self::Workers,
            Self::Watchdog => Self::Problems,
            Self::Events => Self::Watchdog,
            Self::Supervisor => Self::Events,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DashboardState {
    pub active_section: SidebarTab,
    pub scroll: u16,
}

impl Default for DashboardState {
    fn default() -> Self {
        Self {
            active_section: SidebarTab::Workers,
            scroll: 0,
        }
    }
}

impl DashboardState {
    pub fn next_section(&mut self) {
        self.active_section = self.active_section.next();
        self.reset_scroll();
    }

    pub fn previous_section(&mut self) {
        self.active_section = self.active_section.previous();
        self.reset_scroll();
    }

    pub fn select_section(&mut self, section: SidebarTab) {
        self.active_section = section;
        self.reset_scroll();
    }

    pub fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_sub(1);
    }

    pub fn scroll_down(&mut self) {
        self.scroll = self.scroll.saturating_add(1);
    }

    pub fn reset_scroll(&mut self) {
        self.scroll = 0;
    }
}
