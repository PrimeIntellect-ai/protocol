use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event as CrosstermEvent, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame, Terminal,
};
use std::{
    collections::VecDeque,
    io,
    sync::{Arc, Mutex, RwLock},
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
use tracing::{
    field::{Field, Visit},
    Event as TracingEvent, Level, Subscriber,
};
use tracing_subscriber::{layer::Context, Layer};

#[derive(Debug, Clone)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Success,
    Progress,
}

impl From<&Level> for LogLevel {
    fn from(level: &Level) -> Self {
        match *level {
            Level::TRACE => LogLevel::Trace,
            Level::DEBUG => LogLevel::Debug,
            Level::INFO => LogLevel::Info,
            Level::WARN => LogLevel::Warn,
            Level::ERROR => LogLevel::Error,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LogMessage {
    pub level: LogLevel,
    pub message: String,
    pub timestamp: Instant,
    pub target: Option<String>,
}

#[derive(Clone)]
pub struct AppState {
    pub current_task: String,
    pub compute_pool_name: String,
    pub current_reward: String,
    pub logs: Arc<RwLock<VecDeque<LogMessage>>>,
    pub is_running: bool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            current_task: "None".to_string(),
            compute_pool_name: "None".to_string(),
            current_reward: "0.0".to_string(),
            logs: Arc::new(RwLock::new(VecDeque::with_capacity(1000))),
            is_running: true,
        }
    }
}

impl AppState {
    pub fn add_log(&self, level: LogLevel, message: String, target: Option<String>) {
        if let Ok(mut logs) = self.logs.write() {
            logs.push_back(LogMessage {
                level,
                message,
                timestamp: Instant::now(),
                target,
            });
            while logs.len() > 1000 {
                logs.pop_front();
            }
        }
    }

    pub fn set_current_task(&mut self, task: String) {
        self.current_task = task;
    }

    pub fn set_compute_pool_name(&mut self, pool_name: String) {
        self.compute_pool_name = pool_name;
    }

    pub fn set_current_reward(&mut self, reward: String) {
        self.current_reward = reward;
    }
}

// A unified event type for the TUI channel
#[derive(Debug, Clone)]
pub enum TuiEvent {
    Log(LogMessage),
    Input(event::KeyEvent),
}

pub struct Console {
    state: Arc<Mutex<AppState>>,
    log_sender: Option<mpsc::UnboundedSender<TuiEvent>>,
    _tui_active: bool,
}

// Helper visitor to extract formatted messages from tracing events
struct EventVisitor<'a>(&'a mut String);
impl<'a> Visit for EventVisitor<'a> {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            *self.0 = format!("{:?}", value);
        } else {
            if !self.0.is_empty() {
                self.0.push(' ');
            }
            self.0.push_str(&format!("{}={:?}", field.name(), value));
        }
    }
}

// Custom tracing layer that captures logs for the TUI
#[derive(Clone)]
pub struct TuiTracingLayer {
    state: Arc<Mutex<AppState>>,
    log_sender: Option<mpsc::UnboundedSender<TuiEvent>>,
}

impl TuiTracingLayer {
    pub fn new(state: Arc<Mutex<AppState>>) -> Self {
        Self {
            state,
            log_sender: None,
        }
    }

    pub fn set_sender(&mut self, sender: mpsc::UnboundedSender<TuiEvent>) {
        self.log_sender = Some(sender);
    }

    pub fn set_state(&mut self, state: Arc<Mutex<AppState>>) {
        self.state = state;
    }
}

impl<S> Layer<S> for TuiTracingLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &TracingEvent<'_>, _ctx: Context<'_, S>) {
        let mut message = String::new();
        event.record(&mut EventVisitor(&mut message));
        message = message.trim_matches('"').to_string();
        if message.is_empty() {
            message = event.metadata().name().to_string();
        }

        let log_msg = LogMessage {
            level: event.metadata().level().into(),
            message,
            timestamp: Instant::now(),
            target: Some(event.metadata().target().to_string()),
        };

        if let Some(sender) = &self.log_sender {
            let _ = sender.send(TuiEvent::Log(log_msg.clone()));
        }

        if let Ok(state) = self.state.lock() {
            state.add_log(log_msg.level, log_msg.message, log_msg.target);
        }
    }
}

impl Console {
    pub fn new() -> Self {
        let state = Arc::new(Mutex::new(AppState::default()));
        if let Ok(mut layer) = TUI_TRACING_LAYER.write() {
            layer.set_state(state.clone());
        }
        Self {
            state,
            log_sender: None,
            _tui_active: false,
        }
    }

    pub fn start_tui(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        self.log_sender = Some(event_tx.clone());

        if let Ok(mut layer) = TUI_TRACING_LAYER.write() {
            layer.set_sender(event_tx.clone());
        }

        let key_event_tx = event_tx.clone();
        std::thread::spawn(move || loop {
            if let Ok(CrosstermEvent::Key(key)) = event::read() {
                if key_event_tx.send(TuiEvent::Input(key)).is_err() {
                    break;
                }
            }
        });

        let state = self.state.clone();
        self._tui_active = true;

        tokio::spawn(async move {
            let tick_rate = Duration::from_millis(250);
            loop {
                terminal
                    .draw(|f| {
                        if let Ok(state) = state.lock() {
                            ui(f, &state);
                        }
                    })
                    .unwrap();

                match tokio::time::timeout(tick_rate, event_rx.recv()).await {
                    Ok(Some(TuiEvent::Log(log_msg))) => {
                        if let Ok(state) = state.lock() {
                            state.add_log(log_msg.level, log_msg.message, log_msg.target);
                        }
                    }
                    Ok(Some(TuiEvent::Input(key))) => {
                        if key.code == KeyCode::Char('q') || key.code == KeyCode::Esc {
                            break;
                        }
                    }
                    Ok(None) | Err(_) => {
                        // Channel closed or timeout, continue to redraw
                    }
                }
            }

            disable_raw_mode().unwrap();
            execute!(
                terminal.backend_mut(),
                LeaveAlternateScreen,
                DisableMouseCapture
            )
            .unwrap();
            terminal.show_cursor().unwrap();
        });

        Ok(())
    }

    pub fn test_tui_blocking() -> Result<(), Box<dyn std::error::Error>> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let state = AppState::default();
        state.add_log(
            LogLevel::Info,
            "🎨 TUI Test Started".to_string(),
            Some("test".to_string()),
        );
        state.add_log(
            LogLevel::Success,
            "✓ This is a success message".to_string(),
            Some("test".to_string()),
        );
        state.add_log(
            LogLevel::Warn,
            "⚠ This is a warning message".to_string(),
            Some("test".to_string()),
        );
        state.add_log(
            LogLevel::Error,
            "✗ This is an error message".to_string(),
            Some("test".to_string()),
        );

        loop {
            terminal.draw(|f| {
                ui(f, &state);
            })?;
            if event::poll(Duration::from_millis(100))? {
                if let CrosstermEvent::Key(key) = event::read()? {
                    if key.code == KeyCode::Char('q') || key.code == KeyCode::Esc {
                        break;
                    }
                }
            }
        }

        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;
        Ok(())
    }

    pub fn section(title: &str) {
        Self::log_to_tui(
            LogLevel::Info,
            format!("=== {} ===", title),
            Some("console".to_string()),
        );
    }
    pub fn title(text: &str) {
        Self::log_to_tui(
            LogLevel::Info,
            text.to_string(),
            Some("console".to_string()),
        );
    }
    pub fn info(label: &str, value: &str) {
        Self::log_to_tui(
            LogLevel::Info,
            format!("{}: {}", label, value),
            Some("console".to_string()),
        );
    }
    pub fn success(text: &str) {
        let msg = format!("✓ {}", text);
        Self::log_to_tui(LogLevel::Success, msg.clone(), Some("console".to_string()));
    }
    pub fn warning(text: &str) {
        let msg = format!("⚠ {}", text);
        Self::log_to_tui(LogLevel::Warn, msg.clone(), Some("console".to_string()));
    }
    pub fn user_error(text: &str) {
        let msg = format!("✗ {}", text);
        Self::log_to_tui(LogLevel::Error, msg.clone(), Some("console".to_string()));
    }
    pub fn progress(text: &str) {
        let msg = format!("→ {}", text);
        Self::log_to_tui(LogLevel::Progress, msg.clone(), Some("console".to_string()));
    }

    pub fn update_task(&self, task: String) {
        if let Ok(mut state) = self.state.lock() {
            state.set_current_task(task);
        }
    }
    pub fn update_compute_pool(&self, pool_name: String) {
        if let Ok(mut state) = self.state.lock() {
            state.set_compute_pool_name(pool_name);
        }
    }
    pub fn update_reward(&self, reward: String) {
        if let Ok(mut state) = self.state.lock() {
            state.set_current_reward(reward);
        }
    }

    pub fn log_to_tui(level: LogLevel, message: String, target: Option<String>) {
        if let Ok(console) = CONSOLE_INSTANCE.lock() {
            if let Some(sender) = &console.log_sender {
                let _ = sender.send(TuiEvent::Log(LogMessage {
                    level,
                    message,
                    timestamp: Instant::now(),
                    target,
                }));
            }
        }
    }
}

fn ui(f: &mut Frame, app: &AppState) {
    let size = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7), // Logo section
            Constraint::Length(8), // Status section
            Constraint::Length(3), // Disclaimer section
            Constraint::Min(10),   // Logs section
        ])
        .split(size);
    render_logo(f, chunks[0]);
    render_status(f, chunks[1], app);
    render_disclaimer(f, chunks[2]);
    render_logs(f, chunks[3], app);
}
fn render_logo(f: &mut Frame, area: Rect) {
    let logo_text = vec![
        Line::from(vec![Span::styled(
            "██████╗ ██████╗ ██╗███╗   ███╗███████╗",
            Style::default()
            .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![Span::styled(
            "██╔══██╗██╔══██╗██║████╗ ████║██╔════╝",
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![Span::styled(
            "██████╔╝██████╔╝██║██╔████╔██║█████╗  ",
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![Span::styled(
            "██╔═══╝ ██╔══██╗██║██║╚██╔╝██║██╔══╝  ",
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![Span::styled(
            "██║     ██║  ██║██║██║ ╚═╝ ██║███████╗",
            Style::default()
            .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![Span::styled(
            "╚═╝     ╚═╝  ╚═╝╚═╝╚═╝     ╚═╝╚══════╝",
            Style::default()
            .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        )]),
    ];
    let logo = Paragraph::new(logo_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .style(Style::default().fg(Color::DarkGray)),
        )
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true });
    f.render_widget(logo, area);
}
fn render_status(f: &mut Frame, area: Rect, app: &AppState) {
    let status_text = vec![
        Line::from(vec![
            Span::styled(
                "Current Task: ",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(&app.current_task, Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled(
                "Current Compute Pool: ",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                app.compute_pool_name.to_string(),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "Current Reward: ",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                app.current_reward.to_string(),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "Status: ",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                if app.is_running {
                    "● Running"
                } else {
                    "● Stopped"
                },
                Style::default().fg(if app.is_running {
                    Color::White
                } else {
                    Color::DarkGray
                }),
            ),
        ]),
        Line::from(vec![Span::styled(
            "Press 'q' or 'Esc' to quit",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )]),
    ];
    let status = Paragraph::new(status_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Status")
                .style(Style::default().fg(Color::DarkGray)),
        )
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: true });
    f.render_widget(status, area);
}
fn render_disclaimer(f: &mut Frame, area: Rect) {
    let disclaimer_text = vec![Line::from(vec![Span::styled(
        "Any compute contributed to this pool is purely a donation and for testing purposes only.",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )])];
    let disclaimer = Paragraph::new(disclaimer_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .style(Style::default().fg(Color::DarkGray)),
        )
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true });
    f.render_widget(disclaimer, area);
}
fn render_logs(f: &mut Frame, area: Rect, app: &AppState) {
    let logs = match app.logs.read() {
        Ok(logs) => logs,
        Err(_) => {
            let empty_widget = List::new(Vec::<ListItem>::new()).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Logs (Error)")
                    .style(Style::default().fg(Color::DarkGray)),
            );
            f.render_widget(empty_widget, area);
            return;
        }
    };
    // Take only the most recent logs to fit the display area
    let max_logs = area.height.saturating_sub(2) as usize; // Account for borders
    let recent_logs: Vec<_> = if logs.len() > max_logs {
        logs.iter().skip(logs.len() - max_logs).collect()
    } else {
        logs.iter().collect()
    };

    let items: Vec<ListItem> = recent_logs
        .iter()
        .map(|log| {
            let (style, icon) = match log.level {
                LogLevel::Trace | LogLevel::Debug => (Style::default().fg(Color::DarkGray), "•"),
                LogLevel::Info => (Style::default().fg(Color::White), "•"),
                LogLevel::Warn => (Style::default().fg(Color::White), "!"),
                LogLevel::Error => (Style::default().fg(Color::White), "✗"),
                LogLevel::Success => (Style::default().fg(Color::White), "✓"),
                LogLevel::Progress => (Style::default().fg(Color::White), "→"),
            };
            let elapsed = log.timestamp.elapsed();
            let time_str = if elapsed.as_secs() < 60 {
                format!("{}s ago", elapsed.as_secs())
            } else {
                format!("{}m ago", elapsed.as_secs() / 60)
            };
            let target_str = if let Some(target) = &log.target {
                if target != "worker" && target.len() < 20 {
                    format!("[{}] ", target)
                } else {
                    String::new()
                }
            } else {
                String::new()
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("[{}] ", time_str), Style::default().fg(Color::DarkGray)),
                Span::styled(format!("{} ", icon), style.add_modifier(Modifier::BOLD)),
                Span::styled(target_str, Style::default().fg(Color::DarkGray)),
                Span::styled(&log.message, style),
            ]))
        })
        .collect();

    let logs_widget = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Logs")
            .style(Style::default().fg(Color::DarkGray)),
    );

    // Always render without state to show the newest logs at the bottom
    f.render_widget(logs_widget, area);
}

// Global instances
lazy_static::lazy_static! {
    static ref TUI_TRACING_LAYER: RwLock<TuiTracingLayer> = {
        let state = Arc::new(Mutex::new(AppState::default()));
        RwLock::new(TuiTracingLayer::new(state))
    };
    static ref CONSOLE_INSTANCE: Mutex<Console> = Mutex::new(Console::new());
}

impl Console {
    pub fn get_instance() -> &'static Mutex<Console> {
        &CONSOLE_INSTANCE
    }
}
