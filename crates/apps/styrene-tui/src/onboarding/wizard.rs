//! Wizard state machine — drives the onboarding flow.
//!
//! The wizard is a linear sequence of steps with back-navigation. Each step
//! renders via `screens.rs` and handles keyboard input here. The state machine
//! produces a `SetupResult` when the user completes all steps.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;

use crate::tui::theme::Theme;

use super::detect::EnvironmentReport;
use super::reticulum::ReticulumInterface;
use super::setup::{DaemonMode, IdentitySource, SetupResult};

/// Which step of the wizard is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardStep {
    Welcome,
    Identity,
    Profile,
    Network,
    ImportContacts,
    DaemonStart,
    Summary,
}

/// What the wizard loop should do after processing a key event.
pub enum WizardAction {
    /// Key consumed, stay on current step.
    Continue,
    /// Wizard completed — apply results and start the TUI.
    Complete(SetupResult),
    /// User requested quit (Esc from Welcome).
    Quit,
}

/// Full wizard state, including environment report and user choices.
pub struct WizardState {
    pub step: WizardStep,
    pub env: EnvironmentReport,

    // ── Active steps (computed from environment) ────────────────────────────
    active_steps: Vec<WizardStep>,

    // ── User choices ────────────────────────────────────────────────────────
    pub identity_source: IdentitySource,
    pub display_name: String,
    pub node_role: styrened::config::NodeRole,
    pub auto_discover: bool,
    pub imported_interfaces: Vec<(ReticulumInterface, bool)>,
    pub hub_address: String,
    pub imported_contacts: Vec<(String, String, bool)>,
    pub daemon_mode: DaemonMode,

    // ── UI state ────────────────────────────────────────────────────────────
    /// Which focusable element is selected on the current step.
    pub focus: usize,
    /// Cursor position within text input fields.
    pub cursor: usize,
}

impl WizardState {
    pub fn new(env: EnvironmentReport) -> Self {
        // Build the active step list based on what was detected.
        let mut active_steps = vec![
            WizardStep::Welcome,
            WizardStep::Identity,
            WizardStep::Profile,
            WizardStep::Network,
        ];

        // Only show contacts import if there are contacts to import.
        if env.has_importable_contacts() {
            active_steps.push(WizardStep::ImportContacts);
        }

        // Skip daemon start if daemon is already running.
        if !env.daemon_responsive {
            active_steps.push(WizardStep::DaemonStart);
        }

        active_steps.push(WizardStep::Summary);

        // Pre-populate choices from detected environment.
        let identity_source = if env.reticulum.is_some() {
            // Default to import if Reticulum identity exists
            IdentitySource::ImportReticulum(
                env.reticulum.as_ref().map(|r| r.identity_path.clone()).unwrap_or_default(),
            )
        } else {
            IdentitySource::CreateNew
        };

        let imported_interfaces = env
            .reticulum
            .as_ref()
            .map(|r| {
                r.interfaces
                    .iter()
                    .filter(|i| i.enabled)
                    .cloned()
                    .map(|i| (i, true)) // pre-select enabled interfaces
                    .collect()
            })
            .unwrap_or_default();

        let imported_contacts: Vec<(String, String, bool)> = env
            .reticulum
            .as_ref()
            .map(|r| {
                r.known_destinations
                    .iter()
                    .map(|(hash, name)| (hash.clone(), name.clone(), true))
                    .collect()
            })
            .unwrap_or_default();

        let daemon_mode =
            if env.daemon_responsive { DaemonMode::ConnectExisting } else { DaemonMode::Embedded };

        Self {
            step: WizardStep::Welcome,
            env,
            active_steps,
            identity_source,
            display_name: String::new(),
            node_role: styrened::config::NodeRole::FullNode,
            auto_discover: true,
            imported_interfaces,
            hub_address: String::new(),
            imported_contacts,
            daemon_mode,
            focus: 0,
            cursor: 0,
        }
    }

    /// Render the current wizard step.
    pub fn draw(&self, f: &mut Frame, theme: &dyn Theme) {
        super::screens::draw(self, f, theme);
    }

    /// Current step index within the active steps list.
    fn step_index(&self) -> usize {
        self.active_steps.iter().position(|s| *s == self.step).unwrap_or(0)
    }

    /// Total number of active steps.
    pub fn step_count(&self) -> usize {
        self.active_steps.len()
    }

    /// Current step number (1-based, for display).
    pub fn step_number(&self) -> usize {
        self.step_index() + 1
    }

    /// Number of focusable elements on the current step.
    fn focus_count(&self) -> usize {
        match self.step {
            WizardStep::Welcome => 1, // Continue button
            WizardStep::Identity => {
                if self.env.reticulum.is_some() {
                    2
                } else {
                    1
                }
            }
            WizardStep::Profile => 4, // name input + 3 roles
            WizardStep::Network => {
                1 + self.imported_interfaces.len() + 1 // auto-discover + interfaces + hub input
            }
            WizardStep::ImportContacts => {
                1 + self.imported_contacts.len() // select-all + contacts
            }
            WizardStep::DaemonStart => 3, // embedded, background, connect
            WizardStep::Summary => 1,     // Launch button
        }
    }

    /// Advance to the next step.
    fn next_step(&mut self) {
        let idx = self.step_index();
        if idx + 1 < self.active_steps.len() {
            self.step = self.active_steps[idx + 1];
            self.focus = 0;
            self.cursor = 0;
        }
    }

    /// Go back to the previous step.
    fn prev_step(&mut self) {
        let idx = self.step_index();
        if idx > 0 {
            self.step = self.active_steps[idx - 1];
            self.focus = 0;
            self.cursor = 0;
        }
    }

    /// Build the final SetupResult from collected choices.
    fn build_result(&self) -> SetupResult {
        let mut interfaces: Vec<styrened::config::InterfaceConfig> = Vec::new();

        // Auto-discover: local TCP server
        if self.auto_discover {
            interfaces.push(styrened::config::InterfaceConfig {
                kind: "tcp_server".into(),
                enabled: Some(true),
                host: Some("127.0.0.1".into()),
                port: Some(0), // OS-assigned
                name: Some("local-auto".into()),
            });
        }

        // Imported Reticulum interfaces
        for (iface, selected) in &self.imported_interfaces {
            if !*selected {
                continue;
            }
            if let Some(kind) = super::reticulum::map_interface_kind(&iface.kind) {
                interfaces.push(styrened::config::InterfaceConfig {
                    kind: kind.into(),
                    enabled: Some(true),
                    host: iface.host.clone(),
                    port: iface.port,
                    name: Some(iface.name.clone()),
                });
            }
        }

        // Hub address (if provided)
        if !self.hub_address.is_empty() {
            let (host, port) = parse_host_port(&self.hub_address);
            interfaces.push(styrened::config::InterfaceConfig {
                kind: "tcp_client".into(),
                enabled: Some(true),
                host: Some(host),
                port: Some(port),
                name: Some("hub".into()),
            });
        }

        let contacts: Vec<(String, String)> = self
            .imported_contacts
            .iter()
            .filter(|(_, _, selected)| *selected)
            .map(|(hash, name, _)| (hash.clone(), name.clone()))
            .collect();

        SetupResult {
            identity_source: self.identity_source.clone(),
            display_name: self.display_name.clone(),
            node_role: self.node_role,
            interfaces,
            daemon_mode: self.daemon_mode,
            contacts,
        }
    }

    /// Handle a key event. Returns what the wizard loop should do next.
    pub fn handle_key(&mut self, key: KeyEvent) -> WizardAction {
        match key.code {
            // ── Global navigation ───────────────────────────────────────────
            KeyCode::Esc => {
                if self.step == WizardStep::Welcome {
                    return WizardAction::Quit;
                }
                self.prev_step();
            }

            // ── Enter: advance or complete ──────────────────────────────────
            KeyCode::Enter => {
                if self.step == WizardStep::Summary {
                    return WizardAction::Complete(self.build_result());
                }
                // On text input fields, Enter advances to next step
                // On radio/checkbox fields, Enter toggles then advances
                self.next_step();
            }

            // ── Tab / arrow: cycle focus ────────────────────────────────────
            KeyCode::Tab | KeyCode::Down => {
                let count = self.focus_count();
                if count > 0 {
                    self.focus = (self.focus + 1) % count;
                }
            }
            KeyCode::BackTab | KeyCode::Up => {
                let count = self.focus_count();
                if count > 0 {
                    self.focus = if self.focus == 0 { count - 1 } else { self.focus - 1 };
                }
            }

            // ── Space: toggle checkbox/radio ────────────────────────────────
            KeyCode::Char(' ') => {
                self.handle_toggle();
            }

            // ── Text input ──────────────────────────────────────────────────
            KeyCode::Char(c) => {
                self.handle_char(c);
            }
            KeyCode::Backspace => {
                self.handle_backspace();
            }

            _ => {}
        }

        WizardAction::Continue
    }

    fn handle_toggle(&mut self) {
        match self.step {
            WizardStep::Identity => {
                // Toggle between CreateNew and ImportReticulum
                if self.focus == 0 {
                    self.identity_source = IdentitySource::CreateNew;
                } else if let Some(ref rns) = self.env.reticulum {
                    self.identity_source =
                        IdentitySource::ImportReticulum(rns.identity_path.clone());
                }
            }
            WizardStep::Profile => {
                // Focus 0 is name input, 1-3 are role radios
                match self.focus {
                    1 => self.node_role = styrened::config::NodeRole::FullNode,
                    2 => self.node_role = styrened::config::NodeRole::PropagationClient,
                    3 => self.node_role = styrened::config::NodeRole::Hub,
                    _ => {}
                }
            }
            WizardStep::Network => {
                if self.focus == 0 {
                    self.auto_discover = !self.auto_discover;
                } else {
                    let iface_idx = self.focus - 1;
                    if iface_idx < self.imported_interfaces.len() {
                        self.imported_interfaces[iface_idx].1 =
                            !self.imported_interfaces[iface_idx].1;
                    }
                    // Last position is hub input — no toggle
                }
            }
            WizardStep::ImportContacts => {
                if self.focus == 0 {
                    // Select/deselect all
                    let all_selected = self.imported_contacts.iter().all(|(_, _, s)| *s);
                    for contact in &mut self.imported_contacts {
                        contact.2 = !all_selected;
                    }
                } else {
                    let idx = self.focus - 1;
                    if idx < self.imported_contacts.len() {
                        self.imported_contacts[idx].2 = !self.imported_contacts[idx].2;
                    }
                }
            }
            WizardStep::DaemonStart => {
                self.daemon_mode = match self.focus {
                    0 => DaemonMode::Embedded,
                    1 => DaemonMode::Background,
                    2 => DaemonMode::ConnectExisting,
                    _ => self.daemon_mode,
                };
            }
            _ => {}
        }
    }

    fn handle_char(&mut self, c: char) {
        match self.step {
            WizardStep::Profile if self.focus == 0 => {
                if self.display_name.len() < 64 {
                    self.display_name.push(c);
                }
            }
            WizardStep::Network => {
                let hub_idx = 1 + self.imported_interfaces.len();
                if self.focus == hub_idx {
                    self.hub_address.push(c);
                }
            }
            _ => {
                // Space on non-text fields acts as toggle
                if c == ' ' {
                    self.handle_toggle();
                }
            }
        }
    }

    fn handle_backspace(&mut self) {
        match self.step {
            WizardStep::Profile if self.focus == 0 => {
                self.display_name.pop();
            }
            WizardStep::Network => {
                let hub_idx = 1 + self.imported_interfaces.len();
                if self.focus == hub_idx {
                    self.hub_address.pop();
                }
            }
            _ => {}
        }
    }
}

/// Parse "host:port" with a default port of 4242.
fn parse_host_port(addr: &str) -> (String, u16) {
    if let Some((host, port_str)) = addr.rsplit_once(':') {
        if let Ok(port) = port_str.parse::<u16>() {
            return (host.to_string(), port);
        }
    }
    (addr.to_string(), 4242)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_env() -> EnvironmentReport {
        EnvironmentReport::default()
    }

    #[test]
    fn fresh_wizard_has_correct_steps() {
        let wizard = WizardState::new(fresh_env());
        // Fresh: Welcome, Identity, Profile, Network, DaemonStart, Summary
        // No ImportContacts (nothing to import), no skip of DaemonStart (not running)
        assert_eq!(wizard.step_count(), 6);
        assert_eq!(wizard.step, WizardStep::Welcome);
    }

    #[test]
    fn next_step_advances() {
        let mut wizard = WizardState::new(fresh_env());
        assert_eq!(wizard.step, WizardStep::Welcome);
        wizard.next_step();
        assert_eq!(wizard.step, WizardStep::Identity);
        wizard.next_step();
        assert_eq!(wizard.step, WizardStep::Profile);
    }

    #[test]
    fn prev_step_goes_back() {
        let mut wizard = WizardState::new(fresh_env());
        wizard.next_step(); // Identity
        wizard.next_step(); // Profile
        wizard.prev_step(); // Identity
        assert_eq!(wizard.step, WizardStep::Identity);
    }

    #[test]
    fn prev_step_at_welcome_stays() {
        let mut wizard = WizardState::new(fresh_env());
        wizard.prev_step();
        assert_eq!(wizard.step, WizardStep::Welcome);
    }

    #[test]
    fn esc_at_welcome_quits() {
        let mut wizard = WizardState::new(fresh_env());
        let action = wizard.handle_key(KeyEvent::from(KeyCode::Esc));
        assert!(matches!(action, WizardAction::Quit));
    }

    #[test]
    fn enter_at_summary_completes() {
        let mut wizard = WizardState::new(fresh_env());
        // Navigate to Summary
        while wizard.step != WizardStep::Summary {
            wizard.next_step();
        }
        let action = wizard.handle_key(KeyEvent::from(KeyCode::Enter));
        assert!(matches!(action, WizardAction::Complete(_)));
    }

    #[test]
    fn parse_host_port_with_port() {
        assert_eq!(parse_host_port("hub.example.com:5555"), ("hub.example.com".into(), 5555));
    }

    #[test]
    fn parse_host_port_default() {
        assert_eq!(parse_host_port("hub.example.com"), ("hub.example.com".into(), 4242));
    }

    #[test]
    fn daemon_mode_skipped_when_responsive() {
        let env = EnvironmentReport { daemon_responsive: true, ..Default::default() };
        let wizard = WizardState::new(env);
        // DaemonStart should not be in active steps
        assert!(!wizard.active_steps.contains(&WizardStep::DaemonStart));
    }
}
