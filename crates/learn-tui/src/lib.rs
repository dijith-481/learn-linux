#![forbid(unsafe_code)]
#![warn(missing_debug_implementations, missing_docs)]

//! Beginner-first full-screen client.

use std::io::{self, Read, Stdout};
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crossterm::{
    cursor,
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
        KeyboardEnhancementFlags, ModifierKeyCode, MouseButton, MouseEventKind,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use frizbee::{Config, Matcher};
use learn_daemon::{ClientError, Daemon, DaemonClient};
use learn_protocol::{Envelope, envelope, event as protocol_event};
use learn_runtime::{RuntimeError, start_wasm_session};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, Padding, Paragraph, Wrap},
};
use tachyonfx::{EffectManager, Interpolation, fx};
use thiserror::Error;

mod theme;
mod vim_dojo;
use theme::{Theme, ThemeId};
use vim_dojo::VimDojo;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppMode {
    Start,
    Lesson,
    Palette,
    Docs,
    Challenges,
    Progress,
    Vim,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaletteCommand {
    Docs,
    Challenges,
    Progress,
    Vim,
    ResetChallenge,
    TogglePet,
    ToggleRain,
    ToggleAnimations,
    SetTheme(ThemeId),
    Quit,
}

#[derive(Debug, Clone, Copy)]
struct PaletteEntry {
    command: PaletteCommand,
    label: &'static str,
    keywords: &'static str,
}

const PALETTE_ENTRIES: [PaletteEntry; 30] = [
    PaletteEntry {
        command: PaletteCommand::Docs,
        label: "Search local docs",
        keywords: "docs man reference search",
    },
    PaletteEntry {
        command: PaletteCommand::Challenges,
        label: "Choose challenge",
        keywords: "lesson objective exercise open",
    },
    PaletteEntry {
        command: PaletteCommand::Progress,
        label: "View progress",
        keywords: "completed attempts graph skill",
    },
    PaletteEntry {
        command: PaletteCommand::Vim,
        label: "Open Vim dojo",
        keywords: "vim hjkl snake editing substitute game",
    },
    PaletteEntry {
        command: PaletteCommand::ResetChallenge,
        label: "Reset current challenge progress",
        keywords: "clear restart undo progress",
    },
    PaletteEntry {
        command: PaletteCommand::TogglePet,
        label: "Toggle terminal pet",
        keywords: "pet companion mascot creature fun",
    },
    PaletteEntry {
        command: PaletteCommand::ToggleRain,
        label: "Toggle content-aware rain",
        keywords: "rain matrix ambient visual background",
    },
    PaletteEntry {
        command: PaletteCommand::ToggleAnimations,
        label: "Toggle calm animations",
        keywords: "animation motion effects reduce motion",
    },
    PaletteEntry {
        command: PaletteCommand::SetTheme(ThemeId::Default),
        label: "Theme: Default",
        keywords: "theme default classic",
    },
    PaletteEntry {
        command: PaletteCommand::SetTheme(ThemeId::Catppuccin),
        label: "Theme: Catppuccin",
        keywords: "theme catppuccin mocha",
    },
    PaletteEntry {
        command: PaletteCommand::SetTheme(ThemeId::Nord),
        label: "Theme: Nord",
        keywords: "theme nord arctic",
    },
    PaletteEntry {
        command: PaletteCommand::SetTheme(ThemeId::TokyoNight),
        label: "Theme: Tokyo Night",
        keywords: "theme tokyo night",
    },
    PaletteEntry {
        command: PaletteCommand::SetTheme(ThemeId::Everforest),
        label: "Theme: Everforest",
        keywords: "theme everforest dark green",
    },
    PaletteEntry {
        command: PaletteCommand::SetTheme(ThemeId::EverforestLight),
        label: "Theme: Everforest Light",
        keywords: "theme everforest light bright",
    },
    PaletteEntry {
        command: PaletteCommand::SetTheme(ThemeId::CatppuccinLatte),
        label: "Theme: Catppuccin Latte",
        keywords: "theme catppuccin latte light bright",
    },
    PaletteEntry {
        command: PaletteCommand::SetTheme(ThemeId::NordLight),
        label: "Theme: Nord Light",
        keywords: "theme nord light snow arctic bright",
    },
    PaletteEntry {
        command: PaletteCommand::SetTheme(ThemeId::TokyoNightLight),
        label: "Theme: Tokyo Night Light",
        keywords: "theme tokyo night light bright",
    },
    PaletteEntry {
        command: PaletteCommand::SetTheme(ThemeId::Gruvbox),
        label: "Theme: Gruvbox",
        keywords: "theme gruvbox retro warm dark",
    },
    PaletteEntry {
        command: PaletteCommand::SetTheme(ThemeId::GruvboxLight),
        label: "Theme: Gruvbox Light",
        keywords: "theme gruvbox light retro warm bright",
    },
    PaletteEntry {
        command: PaletteCommand::SetTheme(ThemeId::Dracula),
        label: "Theme: Dracula",
        keywords: "theme dracula purple vampire",
    },
    PaletteEntry {
        command: PaletteCommand::SetTheme(ThemeId::RosePine),
        label: "Theme: Rosé Pine",
        keywords: "theme rose pine moody elegant",
    },
    PaletteEntry {
        command: PaletteCommand::SetTheme(ThemeId::SolarizedDark),
        label: "Theme: Solarized Dark",
        keywords: "theme solarized dark eclipse",
    },
    PaletteEntry {
        command: PaletteCommand::SetTheme(ThemeId::SolarizedLight),
        label: "Theme: Solarized Light",
        keywords: "theme solarized light sun bright",
    },
    PaletteEntry {
        command: PaletteCommand::SetTheme(ThemeId::OneDark),
        label: "Theme: One Dark",
        keywords: "theme one dark atom",
    },
    PaletteEntry {
        command: PaletteCommand::SetTheme(ThemeId::Monokai),
        label: "Theme: Monokai",
        keywords: "theme monokai classic neon",
    },
    PaletteEntry {
        command: PaletteCommand::SetTheme(ThemeId::Kanagawa),
        label: "Theme: Kanagawa",
        keywords: "theme kanagawa wave japanese",
    },
    PaletteEntry {
        command: PaletteCommand::SetTheme(ThemeId::Ayu),
        label: "Theme: Ayu Dark",
        keywords: "theme ayu dark minimal",
    },
    PaletteEntry {
        command: PaletteCommand::SetTheme(ThemeId::Palenight),
        label: "Theme: Palenight",
        keywords: "theme palenight material purple",
    },
    PaletteEntry {
        command: PaletteCommand::SetTheme(ThemeId::NightOwl),
        label: "Theme: Night Owl",
        keywords: "theme night owl dark blue",
    },
    PaletteEntry {
        command: PaletteCommand::Quit,
        label: "Quit Learn Linux",
        keywords: "exit close stop",
    },
];

fn palette_matches(query: &str) -> Vec<usize> {
    let query = query.trim().to_ascii_lowercase();
    let mut matches = PALETTE_ENTRIES
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| palette_match_score(&query, entry).map(|score| (score, index)))
        .collect::<Vec<_>>();
    matches.sort_by_key(|(score, index)| (*score, *index));
    if let Some((best, _)) = matches.first().copied() {
        matches.retain(|(score, _)| *score == best);
    }
    matches.into_iter().map(|(_, index)| index).collect()
}

fn palette_match_score(query: &str, entry: &PaletteEntry) -> Option<usize> {
    if query.is_empty() {
        return Some(usize::MAX);
    }
    if let Some(score) = palette_literal_match_score(query, entry) {
        return Some(score);
    }
    let transposed_score = adjacent_transpositions(query)
        .into_iter()
        .filter_map(|variant| palette_literal_match_score(&variant, entry))
        .map(|score| score.saturating_add(4))
        .min();
    if transposed_score.is_some() {
        return transposed_score;
    }

    let haystack = format!("{} {}", entry.label, entry.keywords).to_ascii_lowercase();
    let config = Config::default().max_typos(Some(palette_typo_budget(query.chars().count())));
    let mut matcher = Matcher::new(query, &config);
    let fuzzy_match = matcher.match_list(&[haystack]).into_iter().next()?;
    Some(1_000usize.saturating_add(usize::from(u16::MAX.saturating_sub(fuzzy_match.score))))
}

fn palette_literal_match_score(query: &str, entry: &PaletteEntry) -> Option<usize> {
    let label_words = entry.label.split_whitespace().collect::<Vec<_>>();
    let keyword_words = entry.keywords.split_whitespace().collect::<Vec<_>>();
    if label_words
        .first()
        .is_some_and(|word| word.to_ascii_lowercase().starts_with(query))
    {
        return Some(0);
    }
    if label_words
        .iter()
        .any(|word| word.to_ascii_lowercase().starts_with(query))
    {
        return Some(1);
    }
    if keyword_words
        .iter()
        .any(|word| word.to_ascii_lowercase().starts_with(query))
    {
        return Some(2);
    }
    let haystack = format!("{} {}", entry.label, entry.keywords).to_ascii_lowercase();
    if haystack.contains(query) {
        return Some(3);
    }
    None
}

fn adjacent_transpositions(query: &str) -> Vec<String> {
    let characters = query.chars().collect::<Vec<_>>();
    (0..characters.len().saturating_sub(1))
        .map(|index| {
            let mut transposed = characters.clone();
            transposed.swap(index, index + 1);
            transposed.into_iter().collect()
        })
        .collect()
}

fn palette_typo_budget(character_count: usize) -> u16 {
    match character_count {
        0..=2 => 0,
        3..=4 => 1,
        _ => u16::try_from((character_count / 4).clamp(1, 4)).unwrap_or(4),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PanelFocus {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy)]
struct PetState {
    x: i16,
    dx: i16,
    step_elapsed: Duration,
    mood: u8,
    facing_right: bool,
}

#[derive(Debug, Clone, Default)]
struct KeyboardState {
    last: Option<(KeyCode, KeyModifiers, Duration)>,
    recent: Vec<(String, Duration)>,
    typed_at: Vec<Duration>,
}

impl KeyboardState {
    fn record(&mut self, code: KeyCode, modifiers: KeyModifiers, now: Duration) {
        self.last = Some((code, modifiers, now));
        let trail_key = match code {
            KeyCode::Char(character) if !modifiers.contains(KeyModifiers::CONTROL) => {
                if self
                    .typed_at
                    .last()
                    .is_some_and(|last| now.saturating_sub(*last) > Duration::from_secs(2))
                {
                    self.typed_at.clear();
                }
                self.typed_at.push(now);
                Some(if character == ' ' {
                    "Space".to_owned()
                } else {
                    character.to_string()
                })
            }
            KeyCode::Backspace => Some("⌫".to_owned()),
            KeyCode::Enter => Some("↵".to_owned()),
            KeyCode::Tab => Some("Tab".to_owned()),
            KeyCode::Modifier(ModifierKeyCode::LeftControl | ModifierKeyCode::RightControl) => {
                Some("Ctrl".to_owned())
            }
            KeyCode::Modifier(ModifierKeyCode::LeftShift | ModifierKeyCode::RightShift) => {
                Some("Shift".to_owned())
            }
            KeyCode::Modifier(ModifierKeyCode::LeftAlt | ModifierKeyCode::RightAlt) => {
                Some("Alt".to_owned())
            }
            _ => None,
        };
        if let Some(key) = trail_key {
            self.recent.push((key, now));
            if self.recent.len() > 4 {
                self.recent.remove(0);
            }
        }
    }

    fn wpm(&self, now: Duration) -> Option<usize> {
        let first = *self.typed_at.first()?;
        let last = *self.typed_at.last()?;
        let elapsed = last.saturating_sub(first).as_secs_f64();
        if now.saturating_sub(last) > Duration::from_secs(2)
            || self.typed_at.len() < 5
            || elapsed < 1.0
        {
            return None;
        }
        Some(((self.typed_at.len() as f64 / 5.0) * 60.0 / elapsed).round() as usize)
    }

    fn recent_keys(&self, now: Duration) -> Vec<&str> {
        self.recent
            .iter()
            .filter(|(_, at)| now.saturating_sub(*at) < Duration::from_secs(1))
            .map(|(key, _)| key.as_str())
            .collect()
    }

    fn active(&self, label: &str, position: usize, now: Duration) -> bool {
        let Some((code, modifiers, pressed_at)) = self.last else {
            return false;
        };
        if now.saturating_sub(pressed_at) > Duration::from_millis(220) {
            return false;
        }
        match label {
            "Esc" => code == KeyCode::Esc,
            "Back" => code == KeyCode::Backspace,
            "Tab" => code == KeyCode::Tab,
            "Ctrl" => match code {
                KeyCode::Modifier(ModifierKeyCode::RightControl) => position == 7,
                KeyCode::Modifier(ModifierKeyCode::LeftControl) => position == 0,
                _ => position == 0 && modifiers.contains(KeyModifiers::CONTROL),
            },
            "Alt" => match code {
                KeyCode::Modifier(ModifierKeyCode::RightAlt) => position == 4,
                KeyCode::Modifier(ModifierKeyCode::LeftAlt) => position == 2,
                _ => position == 2 && modifiers.contains(KeyModifiers::ALT),
            },
            "Shift" => match code {
                KeyCode::Modifier(ModifierKeyCode::RightShift) => position == 11,
                KeyCode::Modifier(ModifierKeyCode::LeftShift) => position == 0,
                _ => {
                    position == 0
                        && (modifiers.contains(KeyModifiers::SHIFT)
                            || matches!(code, KeyCode::Char(character) if character.is_ascii_uppercase()))
                }
            },
            "Space" => code == KeyCode::Char(' '),
            "Enter" => code == KeyCode::Enter,
            "←" => code == KeyCode::Left,
            "→" => code == KeyCode::Right,
            _ => matches!(code, KeyCode::Char(character)
                if label.eq_ignore_ascii_case(&character.to_string())),
        }
    }
}

const KEYBOARD_HEIGHT: u16 = 15;

fn keyboard_area(panel: Rect, mode: AppMode) -> Option<Rect> {
    if mode != AppMode::Lesson {
        return None;
    }
    let inner = panel_content_area(panel);
    (inner.width >= 48 && inner.height >= KEYBOARD_HEIGHT + 6).then(|| {
        Rect::new(
            inner.x,
            inner.bottom().saturating_sub(KEYBOARD_HEIGHT),
            inner.width,
            KEYBOARD_HEIGHT,
        )
    })
}

fn render_keyboard(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    keyboard: &KeyboardState,
    now: Duration,
    theme: Theme,
) {
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(" KEYBOARD ")
            .style(Style::default().bg(theme.surface))
            .border_style(theme.muted()),
        area,
    );
    let recent = keyboard.recent_keys(now);
    if !recent.is_empty() {
        let mut chips = Vec::new();
        for key in recent {
            chips.push(Span::styled(
                format!(" {key} "),
                Style::default().fg(theme.fg).bg(theme.overlay),
            ));
            chips.push(Span::raw(" "));
        }
        frame.render_widget(
            Paragraph::new(Line::from(chips)),
            Rect::new(
                area.x.saturating_add(2),
                area.y.saturating_add(1),
                area.width.saturating_sub(16),
                1,
            ),
        );
    }
    let speed = keyboard
        .wpm(now)
        .map_or_else(|| "— WPM".to_owned(), |value| format!("{value} WPM"));
    frame.render_widget(
        Paragraph::new(speed)
            .alignment(Alignment::Right)
            .style(Style::default().fg(theme.warn)),
        Rect::new(
            area.right().saturating_sub(12),
            area.y.saturating_add(1),
            10,
            1,
        ),
    );
    const ROWS: [&[&str]; 5] = [
        &[
            "Esc", "1", "2", "3", "4", "5", "6", "7", "8", "9", "0", "-", "=", "Back",
        ],
        &[
            "Tab", "Q", "W", "E", "R", "T", "Y", "U", "I", "O", "P", "[", "]", r"\",
        ],
        &[
            "Caps", "A", "S", "D", "F", "G", "H", "J", "K", "L", ";", "'", "Enter",
        ],
        &[
            "Shift", "Z", "X", "C", "V", "B", "N", "M", ",", ".", "/", "Shift",
        ],
        &[
            "Ctrl", "Meta", "Alt", "Space", "Alt", "Meta", "Menu", "Ctrl", "←", "→",
        ],
    ];
    let base_x = area.x.saturating_add(area.width.saturating_sub(46) / 2);
    for (row, keys) in ROWS.iter().enumerate() {
        let y = area
            .y
            .saturating_add(3 + u16::try_from(row).unwrap_or(0) * 2);
        let stagger = if row == 4 { 0 } else { row.min(3) };
        let mut x = base_x.saturating_add(u16::try_from(stagger).unwrap_or(0));
        for (position, label) in keys.iter().enumerate() {
            let width = match *label {
                "Esc" | "Back" | "Tab" | "Ctrl" | "Menu" | "Meta" => 4,
                "Caps" | "Enter" | "Shift" => 5,
                "Space" => 8,
                "Alt" => 3,
                _ => 2,
            };
            if x.saturating_add(width) >= area.right().saturating_sub(1) {
                break;
            }
            let active = keyboard.active(label, position, now);
            let face = if active {
                Style::default()
                    .fg(theme.selected_fg)
                    .bg(theme.accent)
                    .bold()
            } else {
                Style::default().fg(theme.fg).bg(theme.overlay)
            };
            let shadow = if active { theme.warn } else { theme.backdrop };
            frame.render_widget(
                Paragraph::new(*label)
                    .alignment(Alignment::Center)
                    .style(face),
                Rect::new(x, y, width, 1),
            );
            for offset in 0..width.saturating_sub(1) {
                if let Some(cell) = frame
                    .buffer_mut()
                    .cell_mut((x.saturating_add(1 + offset), y.saturating_add(1)))
                {
                    cell.set_symbol("▀").set_fg(shadow).set_bg(theme.surface);
                }
            }
            x = x.saturating_add(width + 1);
        }
    }
}

const PET_WIDTH: u16 = 14;
const PET_HEIGHT: u16 = 6;
const PET_STEP: Duration = Duration::from_millis(850);

impl Default for PetState {
    fn default() -> Self {
        Self {
            x: 8,
            dx: 1,
            step_elapsed: Duration::ZERO,
            mood: 0,
            facing_right: true,
        }
    }
}

impl PetState {
    fn tick(&mut self, elapsed: Duration, width: u16) {
        self.step_elapsed += elapsed;
        if self.step_elapsed < PET_STEP {
            return;
        }
        self.step_elapsed = Duration::ZERO;
        self.mood = self.mood.wrapping_add(1);
        let max_x = i16::try_from(width.saturating_sub(PET_WIDTH)).unwrap_or(i16::MAX);
        self.x = self.x.clamp(0, max_x.max(0));
        // A cat pauses between steps instead of gliding like a screensaver.
        if self.mood % 4 == 0 {
            return;
        }
        // The pet is a quiet bottom-edge companion, never an obstruction over lesson text.
        self.x = self.x.saturating_add(self.dx);
        self.x = self.x.clamp(0, max_x.max(0));
        if self.dx != 0 {
            self.facing_right = self.dx > 0;
        }
        if self.x <= 0 || self.x >= max_x {
            self.dx = -self.dx;
        }
    }

    fn handle_key(&mut self, code: KeyCode) -> bool {
        let dx = match code {
            KeyCode::Left => -1,
            KeyCode::Right => 1,
            _ => return false,
        };
        self.dx = dx;
        self.facing_right = dx > 0;
        true
    }

    fn animation_frame(&self, frame_count: usize) -> usize {
        if frame_count == 0 {
            return 0;
        }
        let count = u128::try_from(frame_count).unwrap_or(1);
        let index = self.step_elapsed.as_millis().saturating_mul(count) / PET_STEP.as_millis();
        usize::try_from(index)
            .unwrap_or(0)
            .min(frame_count.saturating_sub(1))
    }
}

fn app_leader(code: KeyCode, modifiers: KeyModifiers) -> bool {
    code == KeyCode::Char(' ') && modifiers.contains(KeyModifiers::CONTROL)
}

/// `Ctrl-T` cycles through the built-in themes from any mode.
fn theme_cycle_key(code: KeyCode, modifiers: KeyModifiers) -> bool {
    matches!(code, KeyCode::Char('t' | 'T')) && modifiers.contains(KeyModifiers::CONTROL)
}

fn panel_focus_key(code: KeyCode, modifiers: KeyModifiers) -> Option<PanelFocus> {
    if !modifiers.contains(KeyModifiers::CONTROL) {
        return None;
    }
    match code {
        KeyCode::Char('h' | 'H') => Some(PanelFocus::Left),
        KeyCode::Char('l' | 'L' | 'r' | 'R') => Some(PanelFocus::Right),
        _ => None,
    }
}

/// Run the default TUI.
pub async fn run(daemon: &Daemon, socket: Option<&Path>) -> Result<(), TuiError> {
    let mut terminal = setup_terminal()?;
    let result = app_loop(&mut terminal, daemon, socket).await;
    restore_terminal(&mut terminal)?;
    result
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>, TuiError> {
    enable_raw_mode().map_err(TuiError::Io)?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        cursor::Hide
    )
    .map_err(TuiError::Io)?;
    if crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false) {
        let _ = execute!(
            stdout,
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                    | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES,
            ),
        );
    }
    Terminal::new(CrosstermBackend::new(stdout)).map_err(TuiError::Io)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<(), TuiError> {
    if crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false) {
        let _ = execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags);
    }
    disable_raw_mode().map_err(TuiError::Io)?;
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen,
        cursor::Show
    )
    .map_err(TuiError::Io)?;
    terminal.show_cursor().map_err(TuiError::Io)
}

#[allow(clippy::too_many_lines)]
async fn app_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    daemon: &Daemon,
    socket: Option<&Path>,
) -> Result<(), TuiError> {
    let mut selected_index = 0_usize;
    let mut selected_challenge = daemon.content.challenges.first().cloned();
    let terminal_size = terminal.size().map_err(TuiError::Io)?;
    let mut terminal_columns = terminal_size.width;
    let mut terminal_rows = terminal_size.height;
    let (mut pane_columns, mut pane_rows) = pane_size(terminal_columns, terminal_rows);
    let mut client = match socket {
        Some(path) => Some(connect_client(path).await?),
        None => None,
    };
    let mut rpc_session_id = None;
    if let Some(client) = client.as_mut()
        && let Some(challenge) = selected_challenge.as_ref()
    {
        let hello = client.hello().await?;
        if !hello.runtime_available {
            return Err(TuiError::Client(ClientError::Protocol(
                "daemon reports that the sandbox runtime is unavailable".to_owned(),
            )));
        }
        let opened = client
            .open_session(challenge.spec.id.as_str(), pane_columns, pane_rows)
            .await?;
        rpc_session_id = Some(opened.session_id);
    }
    let mut session = if client.is_none() {
        selected_challenge
            .as_ref()
            .filter(|_| daemon.runtime.usable())
            .map(|challenge| {
                start_wasm_session(
                    challenge.spec.id.as_str(),
                    &challenge.spec.setup,
                    &challenge.spec.environment,
                    pane_columns,
                    pane_rows,
                )
            })
            .transpose()?
    } else {
        None
    };
    let (output_sender, output_receiver) = mpsc::channel::<Vec<u8>>();
    if let Some(session) = session.as_mut() {
        spawn_output_reader(session, &output_sender)?;
    }
    let mut parser = vt100::Parser::new(pane_rows, pane_columns, 1_000);
    let mut effects: EffectManager<()> = EffectManager::default();
    let mut last_frame = Instant::now();
    let mut animation_time = Duration::ZERO;
    let mut progress_started: Option<Duration> = None;
    let mut completed = false;
    let mut score = 0_usize;
    let mut hint_level = 0_usize;
    let mut revealed_hints = [None; 3];
    let mut shell_input = String::new();
    let mut pending_hint_output: Option<Vec<u8>> = None;
    let mut terminal_history = Vec::new();
    let mut course_complete = false;
    let mut animations_enabled = true;
    let mut mode = AppMode::Start;
    let mut start_transition = None;
    let mut focus = PanelFocus::Left;
    // The pet is an optional atmosphere feature; keep the default lesson
    // focused and let learners enable it from the command palette.
    let mut theme_id = ThemeId::Default;
    let mut pet_enabled = false;
    let mut pet = PetState::default();
    let mut keyboard = KeyboardState::default();
    let mut rain_enabled = false;
    let mut palette_index = 0_usize;
    let mut palette_query = String::new();
    let mut vim_dojo = VimDojo::default();
    let mut docs_query = String::new();
    let mut docs_results = Vec::new();
    let mut progress_rows = Vec::new();
    loop {
        let elapsed = last_frame.elapsed();
        last_frame = Instant::now();
        animation_time += elapsed;
        if let Some(progress) = start_transition.as_mut() {
            *progress += elapsed;
            if *progress >= Duration::from_millis(220) {
                start_transition = None;
                mode = AppMode::Lesson;
                focus = PanelFocus::Left;
            }
        }
        if mode == AppMode::Vim {
            vim_dojo.resize_snake(
                terminal_columns.saturating_sub(2),
                terminal_rows.saturating_sub(6),
            );
            vim_dojo.tick(elapsed);
        }
        if pet_enabled {
            pet.tick(elapsed, pane_columns);
        }
        while let Ok(data) = output_receiver.try_recv() {
            observe_command_output(&data, &mut pending_hint_output, &mut hint_level);
            parser.process(&data);
        }
        // `exit` closes the embedded terminal session.  A closed shell is
        // also the end of the TUI's local lesson process; otherwise the
        // renderer would keep drawing the last terminal frame forever.
        let session_closed = if let Some(active) = session.as_mut() {
            active.try_wait()?.is_some()
        } else {
            false
        };
        if session_closed {
            break;
        }
        if let Some(client) = client.as_mut() {
            while let Some(envelope) = client.poll_event(Duration::from_millis(1)).await? {
                if let Some(envelope::Body::Event(event)) = envelope.body.as_ref()
                    && let Some(protocol_event::Kind::TerminalOutput(output)) = event.kind.as_ref()
                {
                    observe_command_output(&output.data, &mut pending_hint_output, &mut hint_level);
                }
                let was_completed = completed;
                process_event(
                    &envelope,
                    &mut parser,
                    &mut completed,
                    animations_enabled,
                    &mut effects,
                    theme_id.theme(),
                );
                if !was_completed && completed {
                    score = score.saturating_add(1);
                }
            }
        }
        if completed && pending_hint_output.is_none() && !course_complete {
            append_terminal_snapshot(
                &mut terminal_history,
                &parser,
                pane_columns,
                theme_id.theme(),
            );
            terminal_history.extend(completion_lines(
                selected_challenge.as_ref(),
                score,
                daemon.content.challenges.len(),
                pane_columns,
                theme_id.theme(),
            ));
            let advanced = advance_to_next_challenge(
                daemon,
                &mut selected_index,
                &mut selected_challenge,
                &mut session,
                &mut rpc_session_id,
                client.as_mut(),
                pane_columns,
                pane_rows,
                &output_sender,
                &mut parser,
                &mut completed,
            )
            .await?;
            if advanced {
                progress_started = Some(animation_time);
                if let Some(challenge) = selected_challenge.as_ref() {
                    terminal_history
                        .extend(next_challenge_history_lines(challenge, theme_id.theme()));
                }
            } else {
                course_complete = true;
                completed = false;
            }
            focus = PanelFocus::Left;
            hint_level = 0;
            revealed_hints = [None; 3];
            pending_hint_output = None;
            shell_input.clear();
        }
        draw_ui(
            terminal,
            daemon,
            selected_challenge.as_ref(),
            selected_index,
            progress_started,
            session.is_some() || rpc_session_id.is_some(),
            mode,
            start_transition,
            hint_level,
            &revealed_hints,
            palette_index,
            &palette_query,
            &docs_query,
            &docs_results,
            &progress_rows,
            &parser,
            &terminal_history,
            focus,
            pet_enabled,
            &pet,
            &keyboard,
            rain_enabled,
            animation_time,
            elapsed,
            &mut effects,
            animations_enabled,
            &vim_dojo,
            theme_id.theme(),
        )?;
        if event::poll(Duration::from_millis(33)).map_err(TuiError::Io)? {
            match event::read().map_err(TuiError::Io)? {
                Event::Key(key) => {
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }
                    keyboard.record(key.code, key.modifiers, animation_time);
                    if let Some(panel) = panel_focus_key(key.code, key.modifiers) {
                        focus = panel;
                        mode = AppMode::Lesson;
                        if panel == PanelFocus::Right && docs_results.is_empty() {
                            refresh_docs(daemon, client.as_mut(), &docs_query, &mut docs_results)
                                .await?;
                        }
                        continue;
                    }
                    if app_leader(key.code, key.modifiers) {
                        if mode == AppMode::Palette {
                            mode = AppMode::Lesson;
                            palette_query.clear();
                        } else {
                            mode = AppMode::Palette;
                            palette_index = 0;
                            palette_query.clear();
                        }
                        continue;
                    }
                    if theme_cycle_key(key.code, key.modifiers) {
                        theme_id = theme_id.next();
                        continue;
                    }
                    if mode == AppMode::Lesson && key.modifiers.contains(KeyModifiers::CONTROL) {
                        if let KeyCode::Char('1'..='3') = key.code {
                            let index = match key.code {
                                KeyCode::Char('1') => 0,
                                KeyCode::Char('2') => 1,
                                _ => 2,
                            };
                            if selected_challenge
                                .as_ref()
                                .is_some_and(|challenge| challenge.spec.hints.get(index).is_some())
                            {
                                revealed_hints[index] = Some(animation_time);
                            }
                            continue;
                        }
                    }
                    if mode == AppMode::Lesson && focus == PanelFocus::Left {
                        pet.handle_key(key.code);
                    }

                    if mode == AppMode::Start {
                        match key.code {
                            KeyCode::Enter | KeyCode::Char(' ') => {
                                if start_transition.is_none() {
                                    start_transition = Some(Duration::ZERO);
                                }
                            }
                            KeyCode::Esc => break,
                            _ => {}
                        }
                        continue;
                    }

                    if mode == AppMode::Palette {
                        let matches = palette_matches(&palette_query);
                        match key.code {
                            KeyCode::Esc => {
                                mode = AppMode::Lesson;
                                palette_query.clear();
                            }
                            KeyCode::Up => palette_index = palette_index.saturating_sub(1),
                            KeyCode::Down => {
                                if !matches.is_empty() {
                                    palette_index =
                                        (palette_index + 1).min(matches.len().saturating_sub(1));
                                }
                            }
                            KeyCode::Backspace => {
                                palette_query.pop();
                                palette_index = 0;
                            }
                            KeyCode::Char(character)
                                if !key.modifiers.contains(KeyModifiers::CONTROL)
                                    && !key.modifiers.contains(KeyModifiers::ALT) =>
                            {
                                palette_query.push(character);
                                palette_index = 0;
                            }
                            KeyCode::Enter => {
                                if let Some(entry_index) = matches.get(palette_index) {
                                    match PALETTE_ENTRIES[*entry_index].command {
                                        PaletteCommand::Docs => {
                                            mode = AppMode::Docs;
                                            refresh_docs(
                                                daemon,
                                                client.as_mut(),
                                                &docs_query,
                                                &mut docs_results,
                                            )
                                            .await?;
                                        }
                                        PaletteCommand::Challenges => {
                                            mode = AppMode::Challenges;
                                        }
                                        PaletteCommand::Progress => {
                                            mode = AppMode::Progress;
                                            progress_rows = progress_rows_for(daemon);
                                        }
                                        PaletteCommand::Vim => {
                                            mode = AppMode::Vim;
                                            focus = PanelFocus::Left;
                                            vim_dojo.reset();
                                        }
                                        PaletteCommand::ResetChallenge => {
                                            if let Some(challenge) = selected_challenge.as_ref() {
                                                daemon.reset_challenge(&challenge.spec.id)?;
                                                completed = false;
                                                course_complete = false;
                                                terminal_history.clear();
                                                hint_level = 0;
                                                revealed_hints = [None; 3];
                                                shell_input.clear();
                                                if client.is_none() {
                                                    session = Some(start_wasm_session(
                                                        challenge.spec.id.as_str(),
                                                        &challenge.spec.setup,
                                                        &challenge.spec.environment,
                                                        pane_columns,
                                                        pane_rows,
                                                    )?);
                                                    if let Some(session) = session.as_mut() {
                                                        spawn_output_reader(
                                                            session,
                                                            &output_sender,
                                                        )?;
                                                    }
                                                    parser = vt100::Parser::new(
                                                        pane_rows,
                                                        pane_columns,
                                                        1_000,
                                                    );
                                                }
                                            }
                                            mode = AppMode::Lesson;
                                        }
                                        PaletteCommand::TogglePet => {
                                            pet_enabled = !pet_enabled;
                                            mode = AppMode::Lesson;
                                        }
                                        PaletteCommand::ToggleRain => {
                                            rain_enabled = !rain_enabled;
                                            mode = AppMode::Lesson;
                                        }
                                        PaletteCommand::ToggleAnimations => {
                                            animations_enabled = !animations_enabled;
                                            mode = AppMode::Lesson;
                                        }
                                        PaletteCommand::SetTheme(selected_theme) => {
                                            theme_id = selected_theme;
                                            mode = AppMode::Lesson;
                                        }
                                        PaletteCommand::Quit => break,
                                    }
                                    palette_query.clear();
                                    palette_index = 0;
                                }
                            }
                            _ => {}
                        }
                        continue;
                    }
                    if mode == AppMode::Vim {
                        if key.code == KeyCode::Esc && vim_dojo.is_menu() {
                            mode = AppMode::Lesson;
                        } else {
                            vim_dojo.handle_key(key.code, key.modifiers);
                        }
                        continue;
                    }
                    if mode == AppMode::Lesson && focus == PanelFocus::Right {
                        match key.code {
                            KeyCode::Esc => {
                                if docs_query.is_empty() {
                                    focus = PanelFocus::Left;
                                } else {
                                    docs_query.clear();
                                    refresh_docs(
                                        daemon,
                                        client.as_mut(),
                                        &docs_query,
                                        &mut docs_results,
                                    )
                                    .await?;
                                }
                            }
                            KeyCode::Backspace => {
                                docs_query.pop();
                                refresh_docs(
                                    daemon,
                                    client.as_mut(),
                                    &docs_query,
                                    &mut docs_results,
                                )
                                .await?;
                            }
                            KeyCode::Enter => {
                                refresh_docs(
                                    daemon,
                                    client.as_mut(),
                                    &docs_query,
                                    &mut docs_results,
                                )
                                .await?;
                            }
                            KeyCode::Char(character)
                                if !key
                                    .modifiers
                                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                            {
                                docs_query.push(character);
                                refresh_docs(
                                    daemon,
                                    client.as_mut(),
                                    &docs_query,
                                    &mut docs_results,
                                )
                                .await?;
                            }
                            _ => {}
                        }
                        continue;
                    }
                    if mode == AppMode::Progress {
                        if key.code == KeyCode::Esc {
                            mode = AppMode::Lesson;
                        }
                        continue;
                    }
                    if mode == AppMode::Challenges {
                        match key.code {
                            KeyCode::Esc => mode = AppMode::Lesson,
                            KeyCode::Up => {
                                selected_index = selected_index.saturating_sub(1);
                            }
                            KeyCode::Down => {
                                selected_index = (selected_index + 1)
                                    .min(daemon.content.challenges.len().saturating_sub(1));
                            }
                            KeyCode::Enter => {
                                selected_challenge =
                                    daemon.content.challenges.get(selected_index).cloned();
                                completed = false;
                                course_complete = false;
                                terminal_history.clear();
                                hint_level = 0;
                                revealed_hints = [None; 3];
                                shell_input.clear();
                                focus = PanelFocus::Left;
                                if let Some(client) = client.as_mut() {
                                    if let Some(session_id) = rpc_session_id.take() {
                                        client.close_session(&session_id).await?;
                                    }
                                    if let Some(challenge) = selected_challenge.as_ref() {
                                        let opened = client
                                            .open_session(
                                                challenge.spec.id.as_str(),
                                                pane_columns,
                                                pane_rows,
                                            )
                                            .await?;
                                        rpc_session_id = Some(opened.session_id);
                                    }
                                } else if let Some(challenge) = selected_challenge.as_ref() {
                                    session = Some(start_wasm_session(
                                        challenge.spec.id.as_str(),
                                        &challenge.spec.setup,
                                        &challenge.spec.environment,
                                        pane_columns,
                                        pane_rows,
                                    )?);
                                    if let Some(session) = session.as_mut() {
                                        spawn_output_reader(session, &output_sender)?;
                                    }
                                    parser = vt100::Parser::new(pane_rows, pane_columns, 1_000);
                                }
                                mode = AppMode::Lesson;
                            }
                            _ => {}
                        }
                        continue;
                    }
                    if mode == AppMode::Docs {
                        match key.code {
                            KeyCode::Esc => mode = AppMode::Lesson,
                            KeyCode::Backspace => {
                                docs_query.pop();
                                refresh_docs(
                                    daemon,
                                    client.as_mut(),
                                    &docs_query,
                                    &mut docs_results,
                                )
                                .await?;
                            }
                            KeyCode::Char(character) if key.modifiers.is_empty() => {
                                docs_query.push(character);
                                refresh_docs(
                                    daemon,
                                    client.as_mut(),
                                    &docs_query,
                                    &mut docs_results,
                                )
                                .await?;
                            }
                            _ => {}
                        }
                        continue;
                    }
                    match key.code {
                        _ if session.is_some() || rpc_session_id.is_some() => {
                            let submitted_command =
                                track_shell_input(&mut shell_input, key.code, key.modifiers);
                            if submitted_command
                                .as_ref()
                                .is_some_and(|command| command.trim() == "clear")
                            {
                                terminal_history.clear();
                            }
                            if submitted_command
                                .as_ref()
                                .is_some_and(|command| !command.trim().is_empty())
                            {
                                pending_hint_output = Some(Vec::new());
                            }
                            if let Some(bytes) = key_bytes(key.code, key.modifiers) {
                                if let Some(client) = client.as_mut() {
                                    client
                                        .send_terminal_input(
                                            rpc_session_id
                                                .as_deref()
                                                .ok_or(RuntimeError::ReaderTaken)?,
                                            bytes,
                                        )
                                        .await?;
                                } else {
                                    session
                                        .as_ref()
                                        .ok_or(RuntimeError::ReaderTaken)?
                                        .write(&bytes)?;
                                }
                                let verified = client.is_none()
                                    && !completed
                                    && selected_challenge.as_ref().is_some_and(|challenge| {
                                        session.as_ref().is_some_and(|active| {
                                            active.verify(&challenge.spec.verifier).unwrap_or(false)
                                        })
                                    });
                                if verified {
                                    if let Some(challenge) = selected_challenge.as_ref() {
                                        hint_level = 0;
                                        daemon.complete_challenge(&challenge.spec.id)?;
                                    }
                                    completed = true;
                                    score = score.saturating_add(1);
                                } else if submitted_command
                                    .as_ref()
                                    .is_some_and(|command| !command.trim().is_empty())
                                    && !completed
                                {
                                    hint_level = 0;
                                    pending_hint_output = Some(Vec::new());
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Event::Mouse(mouse)
                    if mode == AppMode::Lesson
                        && mouse.kind == MouseEventKind::Down(MouseButton::Left) =>
                {
                    if let Some(index) =
                        hint_click_index(mouse.column, mouse.row, terminal_columns, terminal_rows)
                    {
                        if selected_challenge
                            .as_ref()
                            .is_some_and(|challenge| challenge.spec.hints.get(index).is_some())
                        {
                            revealed_hints[index] = Some(animation_time);
                        }
                    }
                }
                Event::Resize(columns, rows) => {
                    terminal_columns = columns;
                    terminal_rows = rows;
                    (pane_columns, pane_rows) = pane_size(columns, rows);
                    parser.screen_mut().set_size(pane_rows, pane_columns);
                    if let Some(client) = client.as_mut() {
                        client
                            .resize_terminal(
                                rpc_session_id.as_deref().ok_or(RuntimeError::ReaderTaken)?,
                                pane_columns,
                                pane_rows,
                            )
                            .await?;
                    } else if let Some(session) = session.as_ref() {
                        session.resize(pane_columns, pane_rows)?;
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn advance_to_next_challenge(
    daemon: &Daemon,
    selected_index: &mut usize,
    selected_challenge: &mut Option<learn_challenge::CompiledChallenge>,
    session: &mut Option<learn_runtime::WasmSession>,
    rpc_session_id: &mut Option<String>,
    client: Option<&mut DaemonClient>,
    pane_columns: u16,
    pane_rows: u16,
    output_sender: &mpsc::Sender<Vec<u8>>,
    parser: &mut vt100::Parser,
    completed: &mut bool,
) -> Result<bool, TuiError> {
    let next_index = selected_index.saturating_add(1);
    let Some(challenge) = daemon.content.challenges.get(next_index).cloned() else {
        return Ok(false);
    };
    *selected_index = next_index;
    *selected_challenge = Some(challenge.clone());
    *completed = false;
    if let Some(client) = client {
        if let Some(session_id) = rpc_session_id.take() {
            client.close_session(&session_id).await?;
        }
        let opened = client
            .open_session(challenge.spec.id.as_str(), pane_columns, pane_rows)
            .await?;
        *rpc_session_id = Some(opened.session_id);
    } else {
        let history = session
            .as_ref()
            .map(learn_runtime::WasmSession::command_history);
        let next_session = learn_runtime::start_wasm_session_quiet(
            challenge.spec.id.as_str(),
            &challenge.spec.setup,
            &challenge.spec.environment,
            pane_columns,
            pane_rows,
        )?;
        if let Some(history) = history {
            next_session.set_command_history(history)?;
        }
        *session = Some(next_session);
        if let Some(session) = session.as_mut() {
            spawn_output_reader(session, output_sender)?;
        }
    }
    *parser = vt100::Parser::new(pane_rows, pane_columns, 1_000);
    Ok(true)
}

async fn connect_client(path: &Path) -> Result<DaemonClient, TuiError> {
    let mut last_error = None;
    for _ in 0..20 {
        match DaemonClient::connect(path).await {
            Ok(client) => return Ok(client),
            Err(error) => {
                last_error = Some(error);
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        }
    }
    Err(TuiError::Client(
        last_error.expect("client attempts are non-empty"),
    ))
}

fn spawn_output_reader(
    session: &mut learn_runtime::WasmSession,
    output_sender: &mpsc::Sender<Vec<u8>>,
) -> Result<(), TuiError> {
    let mut reader = session.take_reader().ok_or(RuntimeError::ReaderTaken)?;
    let output_sender = output_sender.clone();
    thread::spawn(move || {
        let mut buffer = [0_u8; 4096];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    if output_sender.send(buffer[..count].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    Ok(())
}

async fn refresh_docs(
    daemon: &Daemon,
    client: Option<&mut DaemonClient>,
    query: &str,
    results: &mut Vec<(String, String)>,
) -> Result<(), TuiError> {
    *results = if let Some(client) = client {
        client
            .search_docs(query, 5)
            .await?
            .into_iter()
            .map(|result| (result.title, result.snippet))
            .collect()
    } else {
        daemon
            .search_docs(query, 5)
            .into_iter()
            .map(|result| (result.document.title, result.snippet))
            .collect()
    };
    Ok(())
}

fn progress_rows_for(daemon: &Daemon) -> Vec<String> {
    daemon.progress().map_or_else(
        |error| vec![format!("Progress unavailable: {error}")],
        |progress| {
            if progress.is_empty() {
                vec!["No attempts yet.".to_owned()]
            } else {
                progress
                    .into_iter()
                    .map(|item| {
                        format!(
                            "{:?} {} ({} attempts)",
                            item.status, item.challenge_id, item.attempts
                        )
                    })
                    .collect()
            }
        },
    )
}

fn process_event(
    envelope: &Envelope,
    parser: &mut vt100::Parser,
    completed: &mut bool,
    animations_enabled: bool,
    effects: &mut EffectManager<()>,
    theme: Theme,
) {
    let Some(envelope::Body::Event(event)) = envelope.body.as_ref() else {
        return;
    };
    match event.kind.as_ref() {
        Some(protocol_event::Kind::TerminalOutput(output)) => parser.process(&output.data),
        Some(protocol_event::Kind::ChallengeState(state))
            if state.state == "completed" && !*completed =>
        {
            *completed = true;
            if animations_enabled {
                effects.add_effect(fx::fade_from_fg(
                    theme.accent,
                    (500, Interpolation::QuadOut),
                ));
            }
        }
        _ => {}
    }
}

fn observe_command_output(
    data: &[u8],
    pending_output: &mut Option<Vec<u8>>,
    hint_level: &mut usize,
) {
    let Some(response) = pending_output.as_mut() else {
        return;
    };
    response.extend_from_slice(data);
    if response.len() > 4096 {
        let excess = response.len() - 4096;
        response.drain(..excess);
    }
    if let Some(status) = command_exit_status(response) {
        if status != 0 {
            *hint_level = hint_level.saturating_add(1);
        }
        *pending_output = None;
    } else if response
        .windows(b"\x07learner@learn".len())
        .any(|window| window == b"\x07learner@learn")
    {
        *pending_output = None;
    }
}

fn command_exit_status(output: &[u8]) -> Option<i32> {
    let marker = b"[exit ";
    let start = output
        .windows(marker.len())
        .rposition(|window| window == marker)?
        + marker.len();
    let end = output[start..].iter().position(|byte| *byte == b']')? + start;
    std::str::from_utf8(&output[start..end]).ok()?.parse().ok()
}

fn track_shell_input(input: &mut String, code: KeyCode, modifiers: KeyModifiers) -> Option<String> {
    if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
        input.clear();
        return None;
    }
    match code {
        KeyCode::Enter => {
            let submitted = input.trim().to_owned();
            input.clear();
            Some(submitted)
        }
        KeyCode::Backspace => {
            input.pop();
            None
        }
        KeyCode::Char(character)
            if !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            input.push(character);
            None
        }
        _ => None,
    }
}

#[allow(
    clippy::fn_params_excessive_bools,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
fn draw_ui(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    daemon: &Daemon,
    selected_challenge: Option<&learn_challenge::CompiledChallenge>,
    selected_index: usize,
    progress_started: Option<Duration>,
    session_active: bool,
    mode: AppMode,
    start_transition: Option<Duration>,
    hint_level: usize,
    revealed_hints: &[Option<Duration>; 3],
    palette_index: usize,
    palette_query: &str,
    docs_query: &str,
    docs_results: &[(String, String)],
    progress_rows: &[String],
    parser: &vt100::Parser,
    terminal_history: &[Line<'static>],
    focus: PanelFocus,
    pet_enabled: bool,
    pet: &PetState,
    keyboard: &KeyboardState,
    rain_enabled: bool,
    animation_time: Duration,
    elapsed: Duration,
    effects: &mut EffectManager<()>,
    animations_enabled: bool,
    vim_dojo: &VimDojo,
    theme: Theme,
) -> Result<(), TuiError> {
    terminal
        .draw(|frame| {
            let area = frame.area();
            // Repaint the whole frame so theme changes also update cells that the
            // terminal emulator did not explicitly write this frame.
            frame.render_widget(
                Block::default().style(Style::default().bg(theme.surface)),
                area,
            );
            if mode == AppMode::Start {
                render_start_screen(
                    frame,
                    area,
                    animation_time,
                    start_transition,
                    daemon.content.challenges.len(),
                    theme,
                );
                return;
            }
            if mode == AppMode::Vim && vim_dojo.is_snake() {
                let lines = vim_dojo.fullscreen_lines(area.width, area.height, theme);
                let snake = Paragraph::new(lines)
                    .block(Block::default().padding(Padding::top(1)))
                    .alignment(Alignment::Center);
                frame.render_widget(snake, area);
                return;
            }
            let right_columns = if mode == AppMode::Challenges {
                area.width.saturating_mul(36) / 100
            } else if area.width >= 118 {
                (area.width.saturating_mul(28) / 100).max(52)
            } else {
                area.width.saturating_mul(28) / 100
            };
            let columns = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Min(0), Constraint::Length(right_columns)])
                .split(area);
            let left_sections = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(brief_height(area.height)),
                    Constraint::Min(0),
                ])
                .split(columns[0]);
            let brief_lines = challenge_brief_lines(selected_challenge, theme);
            let runtime_text = daemon.runtime.render();
            let mut terminal_lines = if mode == AppMode::Vim {
                vim_dojo.main_lines(left_sections[1].width.saturating_sub(2), theme)
            } else {
                let mut lines = terminal_history.to_vec();
                if session_active {
                    let width = left_sections[1].width.saturating_sub(2);
                    lines.extend(terminal_lines_with_cursor(
                        parser,
                        width,
                        mode == AppMode::Lesson,
                        theme,
                    ));
                } else {
                    lines.extend(runtime_text.lines().map(Line::from));
                }
                lines
            };
            if mode == AppMode::Lesson && hint_level > 0 {
                if let Some(hint) = selected_challenge.and_then(|challenge| {
                    challenge
                        .spec
                        .hints
                        .get(
                            hint_level
                                .saturating_sub(1)
                                .min(challenge.spec.hints.len().saturating_sub(1)),
                        )
                        .map(|hint| hint.text.as_str())
                }) {
                    let speaker = if pet_enabled { "◉  " } else { "↳  " };
                    terminal_lines.push(Line::from(vec![
                        Span::styled(speaker, theme.accent().add_modifier(Modifier::BOLD)),
                        Span::styled(hint.to_owned(), theme.dim()),
                    ]));
                }
            }
            let brief_block = Block::default()
                .borders(Borders::ALL)
                .style(Style::default().bg(theme.surface))
                .border_style(if focus == PanelFocus::Left {
                    theme.accent()
                } else {
                    theme.muted()
                })
                .title(" YOUR TASK ")
                .padding(Padding::horizontal(1));
            let brief_inner = brief_block.inner(left_sections[0]);
            frame.render_widget(brief_block, left_sections[0]);
            let progress_height = if brief_inner.height >= 5 { 2 } else { 0 };
            let task_area = Rect::new(
                brief_inner.x,
                brief_inner.y,
                brief_inner.width,
                brief_inner.height.saturating_sub(progress_height),
            );
            frame.render_widget(
                Paragraph::new(brief_lines).wrap(Wrap { trim: true }),
                task_area,
            );
            if let Some(challenge) = selected_challenge {
                render_locked_hints(
                    frame,
                    brief_inner,
                    &challenge.spec.hints,
                    revealed_hints,
                    animation_time,
                    animations_enabled,
                    theme,
                );
            }
            if progress_height > 0 {
                render_level_progress(
                    frame,
                    brief_inner,
                    selected_index,
                    daemon.content.challenges.len(),
                    progress_started,
                    animation_time,
                    animations_enabled,
                    theme,
                );
            }
            let terminal_block = Block::default()
                .borders(Borders::ALL)
                .style(Style::default().bg(theme.surface))
                .border_style(if focus == PanelFocus::Left {
                    theme.accent()
                } else {
                    theme.muted()
                })
                .title(" TERMINAL ")
                .padding(Padding::top(1));
            let terminal_view_height = usize::from(left_sections[1].height.saturating_sub(3));
            let terminal_scroll =
                u16::try_from(terminal_lines.len().saturating_sub(terminal_view_height))
                    .unwrap_or(u16::MAX);
            let left = Paragraph::new(terminal_lines)
                .block(terminal_block)
                .scroll((terminal_scroll, 0))
                .wrap(Wrap { trim: false });
            frame.render_widget(left, left_sections[1]);
            if rain_enabled && mode == AppMode::Lesson {
                render_content_rain(
                    frame,
                    left_sections[1],
                    selected_challenge,
                    animation_time,
                    theme,
                );
            }
            if pet_enabled && mode == AppMode::Lesson {
                render_terminal_pet(frame, left_sections[1], pet, theme);
            }
            let mut right_lines = match mode {
                AppMode::Palette => palette_lines(palette_query, palette_index, theme),
                AppMode::Vim => vim_dojo.side_lines(theme),
                AppMode::Challenges => challenge_lines(
                    daemon,
                    selected_index,
                    columns[1].width.saturating_sub(2) as usize,
                    theme,
                ),
                AppMode::Progress => {
                    let mut lines =
                        vec![Line::from(Span::styled("Progress (modal)", theme.accent()))];
                    lines.extend(progress_rows.iter().map(|row| Line::from(row.as_str())));
                    lines
                }
                AppMode::Docs => {
                    let mut lines = vec![Line::from(Span::styled(
                        "Docs search (modal)",
                        theme.accent(),
                    ))];
                    lines.push(Line::from(format!("> {docs_query}")));
                    for (title, snippet) in docs_results {
                        lines.push(Line::from(Span::styled(title.as_str(), theme.accent())));
                        lines.push(Line::from(snippet.as_str()));
                    }
                    lines
                }
                AppMode::Start => Vec::new(),
                AppMode::Lesson => docs_sidebar_lines(docs_query, docs_results, theme),
            };
            let keyboard_rect = keyboard_area(columns[1], mode);
            if let Some(keyboard_rect) = keyboard_rect {
                right_lines.truncate(usize::from(
                    keyboard_rect
                        .y
                        .saturating_sub(columns[1].y.saturating_add(2)),
                ));
            }
            let right_block = Block::default()
                .borders(Borders::ALL)
                .style(Style::default().bg(theme.surface))
                .border_style(if focus == PanelFocus::Right {
                    theme.accent()
                } else {
                    theme.muted()
                })
                .padding(Padding::top(1));
            let right = Paragraph::new(right_lines)
                .block(right_block)
                .wrap(Wrap { trim: true });
            frame.render_widget(right, columns[1]);
            if let Some(keyboard_rect) = keyboard_rect {
                render_keyboard(frame, keyboard_rect, keyboard, animation_time, theme);
            }
            if mode == AppMode::Palette {
                render_palette_overlay(frame, area, palette_query, palette_index, theme);
            }
            effects.process_effects(
                elapsed.into(),
                frame.buffer_mut(),
                panel_content_area(columns[1]),
            );
        })
        .map(|_| ())
        .map_err(TuiError::Io)
}

fn terminal_lines_with_cursor(
    parser: &vt100::Parser,
    width: u16,
    show_cursor: bool,
    theme: Theme,
) -> Vec<Line<'static>> {
    let screen = parser.screen();
    let (cursor_row, cursor_column) = screen.cursor_position();
    let rows = screen.rows(0, width).collect::<Vec<_>>();
    let last_content_row = rows.iter().rposition(|row| !row.trim().is_empty());
    let last_row = last_content_row
        .into_iter()
        .chain(show_cursor.then_some(usize::from(cursor_row)))
        .max();
    let Some(last_row) = last_row else {
        return Vec::new();
    };
    rows.into_iter()
        .take(last_row + 1)
        .enumerate()
        .map(|(row, contents)| {
            if !show_cursor || row != usize::from(cursor_row) {
                return terminal_line_spans(&contents, theme);
            }
            let characters = contents.chars().collect::<Vec<_>>();
            let cursor = usize::from(cursor_column).min(characters.len());
            let before = characters[..cursor].iter().collect::<String>();
            let highlighted = characters.get(cursor).copied().unwrap_or(' ');
            let after = characters.get(cursor.saturating_add(1)..).unwrap_or(&[]);
            let mut spans = terminal_line_spans(&before, theme).spans;
            spans.push(Span::styled(
                highlighted.to_string(),
                Style::default()
                    .fg(theme.selected_fg)
                    .bg(theme.selected_bg)
                    .bold(),
            ));
            spans.extend(terminal_line_spans(&after.iter().collect::<String>(), theme).spans);
            Line::from(spans)
        })
        .collect()
}

fn terminal_line_spans(contents: &str, theme: Theme) -> Line<'static> {
    if let Some(line) = startup_fetch_line(contents, theme) {
        return line;
    }
    if let Some(line) = starship_prompt_line(contents, theme) {
        return line;
    }
    let Some(identity_start) = contents.find("learner@learn") else {
        return Line::from(Span::styled(contents.to_owned(), theme.fg));
    };
    let identity_end = identity_start + "learner@learn".len();
    let Some(operator_offset) = contents[identity_end..].find(" ❯ ") else {
        return Line::from(Span::styled(contents.to_owned(), theme.fg));
    };
    let operator_start = identity_end + operator_offset;
    let path_start = contents[identity_end..operator_start]
        .find(|character: char| !character.is_ascii_whitespace())
        .map_or(operator_start, |offset| identity_end + offset);
    let path_end = identity_end + contents[identity_end..operator_start].trim_end().len();
    let command_start = operator_start + " ❯ ".len();
    let mut spans = Vec::new();
    if identity_start > 0 {
        spans.push(Span::styled(
            contents[..identity_start].to_owned(),
            theme.fg,
        ));
    }
    spans.push(Span::styled(
        contents[identity_start..identity_end].to_owned(),
        theme.accent().add_modifier(Modifier::BOLD),
    ));
    if path_start > identity_end {
        spans.push(Span::styled(
            contents[identity_end..path_start].to_owned(),
            theme.muted(),
        ));
    }
    if path_end > path_start {
        spans.push(Span::styled(
            contents[path_start..path_end].to_owned(),
            Style::default().fg(theme.warn),
        ));
    }
    spans.push(Span::styled(
        contents[path_end..command_start].to_owned(),
        Style::default()
            .fg(theme.success)
            .add_modifier(Modifier::BOLD),
    ));
    if command_start < contents.len() {
        spans.push(Span::styled(contents[command_start..].to_owned(), theme.fg));
    }
    Line::from(spans)
}

fn startup_fetch_line(contents: &str, theme: Theme) -> Option<Line<'static>> {
    if let Some((logo, info)) = contents.split_once("│")
        && logo.chars().any(|character| "▄█▀".contains(character))
    {
        let info = info.trim_start();
        if info.starts_with("🐧  ") {
            return Some(Line::from(vec![
                Span::styled(logo.to_owned(), theme.accent().add_modifier(Modifier::BOLD)),
                Span::styled("│ ", theme.muted()),
                Span::styled(
                    info.to_owned(),
                    Style::default().fg(theme.warn).add_modifier(Modifier::BOLD),
                ),
            ]));
        }
        let label_end = info.find("  ").unwrap_or(info.len());
        return Some(Line::from(vec![
            Span::styled(logo.to_owned(), theme.accent().add_modifier(Modifier::BOLD)),
            Span::styled("│ ", theme.muted()),
            Span::styled(
                info[..label_end].to_owned(),
                Style::default().fg(theme.warn),
            ),
            Span::styled(info[label_end..].to_owned(), theme.fg),
        ]));
    }
    if contents.trim_start().starts_with("Learn Linux") {
        return Some(Line::from(Span::styled(contents.to_owned(), theme.dim())));
    }
    if let Some(rest) = contents.strip_prefix("    runtime  ") {
        let Some((runtime, quest)) = rest.split_once("    quest  ") else {
            return Some(Line::from(Span::styled(contents.to_owned(), theme.dim())));
        };
        return Some(Line::from(vec![
            Span::styled("    runtime  ", theme.accent()),
            Span::styled(runtime.to_owned(), theme.fg),
            Span::styled("    quest  ", Style::default().fg(theme.warn)),
            Span::styled(quest.to_owned(), theme.fg),
        ]));
    }
    None
}

fn starship_prompt_line(contents: &str, theme: Theme) -> Option<Line<'static>> {
    if contents.contains("learner@learn") && contents.contains("  ◈") {
        let identity_start = contents.find("learner@learn")?;
        let identity_end = identity_start + "learner@learn".len();
        let icon_start = contents[identity_end..].find("  ◈")? + identity_end;
        let path_start = contents[identity_end..icon_start]
            .find(|character: char| !character.is_ascii_whitespace())
            .map_or(icon_start, |offset| identity_end + offset);
        let path_end = identity_end + contents[identity_end..icon_start].trim_end().len();
        return Some(Line::from(vec![
            Span::styled(contents[..identity_start].to_owned(), theme.muted()),
            Span::styled(
                contents[identity_start..identity_end].to_owned(),
                theme.accent().add_modifier(Modifier::BOLD),
            ),
            Span::styled(contents[identity_end..path_start].to_owned(), theme.muted()),
            Span::styled(
                contents[path_start..path_end].to_owned(),
                Style::default().fg(theme.warn),
            ),
            Span::styled(contents[path_end..icon_start + 2].to_owned(), theme.muted()),
            Span::styled(
                contents[icon_start + 2..].to_owned(),
                Style::default()
                    .fg(theme.success)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    }
    if let Some(operator_start) = contents.find("❯")
        && (contents.starts_with("╰─") || contents.starts_with("❯"))
    {
        return Some(Line::from(vec![
            Span::styled(contents[..operator_start].to_owned(), theme.muted()),
            Span::styled(
                "❯",
                Style::default()
                    .fg(theme.success)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(contents[operator_start + "❯".len()..].to_owned(), theme.fg),
        ]));
    }
    None
}

fn render_start_screen(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    time: Duration,
    transition: Option<Duration>,
    _challenge_count: usize,
    theme: Theme,
) {
    frame.render_widget(
        Block::default().style(Style::default().bg(theme.surface)),
        area,
    );
    render_game_of_life(frame, area, time, theme);

    let content = centered_rect(72, 74, area);
    let action_style = if transition.is_some() {
        theme.selected().add_modifier(Modifier::BOLD)
    } else {
        theme.selected()
    };
    let mut lines = if area.width >= 44 && area.height >= 16 {
        let mut lines = start_word_lines("LEARN", theme.accent().add_modifier(Modifier::BOLD));
        lines.extend(start_word_lines(
            "LINUX",
            Style::default().fg(theme.warn).add_modifier(Modifier::BOLD),
        ));
        lines
    } else {
        vec![Line::from(Span::styled("LEARN LINUX", theme.title()))]
    };
    lines.extend([
        Line::from(Span::styled("keyboard-first shell dojo", theme.dim())),
        Line::from(""),
        Line::from(Span::styled("[ ENTER ]  START LEARNING", action_style)),
        Line::from(""),
        Line::from(Span::styled(
            "Vim dojo in command center  ·  Ctrl-Space commands  ·  Esc quit",
            theme.dim(),
        )),
    ]);
    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .block(Block::default().padding(Padding::uniform(1))),
        content,
    );
}

fn render_game_of_life(frame: &mut ratatui::Frame<'_>, area: Rect, time: Duration, theme: Theme) {
    let width = usize::from(area.width / 2);
    let height = usize::from(area.height);
    if width < 4 || height < 4 {
        return;
    }
    let mut cells = vec![false; width * height];
    for y in 0..height {
        for x in 0..width {
            let seed = x.wrapping_mul(31).wrapping_add(y.wrapping_mul(17));
            cells[y * width + x] = seed % 29 == 0 || (x + y * 3) % 97 == 0;
        }
    }
    let generations = usize::try_from((time.as_millis() / 420) % 24).unwrap_or(0);
    for _ in 0..generations {
        let mut next = vec![false; cells.len()];
        for y in 0..height {
            for x in 0..width {
                let mut neighbors = 0_u8;
                for dy in -1_i32..=1 {
                    for dx in -1_i32..=1 {
                        if dx == 0 && dy == 0 {
                            continue;
                        }
                        let nx = i32::try_from(x).unwrap_or(0) + dx;
                        let ny = i32::try_from(y).unwrap_or(0) + dy;
                        if nx >= 0
                            && ny >= 0
                            && usize::try_from(nx).is_ok_and(|value| value < width)
                            && usize::try_from(ny).is_ok_and(|value| value < height)
                            && cells[usize::try_from(ny).unwrap_or(0) * width
                                + usize::try_from(nx).unwrap_or(0)]
                        {
                            neighbors += 1;
                        }
                    }
                }
                let alive = cells[y * width + x];
                next[y * width + x] = neighbors == 3 || (alive && neighbors == 2);
            }
        }
        cells = next;
    }
    let buffer = frame.buffer_mut();
    for y in 0..height {
        for x in 0..width {
            if !cells[y * width + x] {
                continue;
            }
            let screen_x = area.x.saturating_add(u16::try_from(x * 2).unwrap_or(0));
            let screen_y = area.y.saturating_add(u16::try_from(y).unwrap_or(0));
            if let Some(cell) = buffer.cell_mut((screen_x, screen_y)) {
                cell.set_symbol(if (x + y) % 5 == 0 { "•" } else { "·" })
                    .set_style(Style::default().fg(if (x + y) % 5 == 0 {
                        theme.muted
                    } else {
                        theme.backdrop
                    }));
            }
        }
    }
}

#[allow(dead_code, clippy::too_many_lines)]
fn render_legacy_start_screen(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    time: Duration,
    transition: Option<Duration>,
    challenge_count: usize,
) {
    let background = Color::Rgb(10, 15, 23);
    frame.render_widget(
        Block::default().style(Style::default().bg(background)),
        area,
    );
    render_start_backdrop(frame, area, time);

    if area.width < 90 || area.height < 26 {
        render_compact_start_screen(frame, area, transition, challenge_count);
        return;
    }

    let margin = if area.width >= 120 { 2 } else { 1 };
    let content = Rect::new(
        area.x.saturating_add(margin),
        area.y.saturating_add(1),
        area.width.saturating_sub(margin.saturating_mul(2)),
        area.height.saturating_sub(2),
    );
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(17),
            Constraint::Length(8),
        ])
        .split(content);

    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            " LEARN",
            Style::default()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  /  LINUX SHELL DOJO", Style::default().fg(Color::Gray)),
        Span::raw(" ".repeat(3)),
        Span::styled("WASM LAB", Style::default().fg(Color::LightGreen)),
        Span::styled(
            "  •  OFFLINE  •  v0.1",
            Style::default().fg(Color::DarkGray),
        ),
    ]))
    .block(Block::default().padding(Padding::bottom(1)));
    frame.render_widget(header, rows[0]);

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(66), Constraint::Percentage(34)])
        .split(rows[1]);

    let progress = start_transition_percent(transition);
    let mut hero_lines = vec![Line::from(Span::styled(
        "  YOUR NEXT SKILL IS WAITING",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    ))];
    hero_lines.extend(start_word_lines(
        "LEARN",
        Style::default()
            .fg(Color::LightCyan)
            .add_modifier(Modifier::BOLD),
    ));
    hero_lines.extend(start_word_lines(
        "LINUX",
        Style::default()
            .fg(Color::LightMagenta)
            .add_modifier(Modifier::BOLD),
    ));
    hero_lines.extend([
        Line::from(Span::styled(
            "  A visual path from pwd to dangerous competence.",
            Style::default().fg(Color::Gray),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  [", Style::default().fg(Color::DarkGray)),
            Span::styled(
                if transition.is_some() {
                    " OPENING FIRST QUEST "
                } else {
                    " ENTER  START LEARNING "
                },
                Style::default()
                    .fg(Color::Black)
                    .bg(if transition.is_some() {
                        Color::Yellow
                    } else {
                        Color::LightGreen
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("]", Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(Span::styled(
            if transition.is_some() {
                format!("  syncing the sandbox  {progress}%")
            } else {
                "  Enter begin  •  Ctrl-Space command center  •  Esc quit".to_owned()
            },
            Style::default().fg(Color::DarkGray),
        )),
    ]);
    let hero = Paragraph::new(hero_lines)
        .alignment(Alignment::Center)
        .style(Style::default().bg(Color::Rgb(13, 21, 31)))
        .block(
            Block::default()
                .title(" ◈  WELCOME, LEARNER ")
                .title_bottom(" keyboard first  //  no mouse required ")
                .padding(Padding::uniform(1)),
        );
    frame.render_widget(hero, columns[0]);

    let mission_width = usize::from(columns[1].width.saturating_sub(6));
    let mut mission = vec![
        Line::from(Span::styled(
            "MISSION CONTROL",
            Style::default()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "FIRST BOOT  /  FOUNDATION",
            Style::default().fg(Color::Yellow),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "10 quests  •  1 skill graph  •  ∞ tiny victories",
            Style::default().fg(Color::Gray),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "PATH STATUS",
            Style::default().fg(Color::DarkGray),
        )),
        start_progress_line(0, challenge_count.max(1), mission_width.min(22)),
        Line::from(Span::styled(
            "[01] Orientation  →  [02] Files  →  [03] Search",
            Style::default().fg(Color::LightGreen),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "TOOLS ONLINE",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(vec![
            Span::styled("● ", Style::default().fg(Color::LightGreen)),
            Span::styled("virtual Linux", Style::default().fg(Color::Gray)),
        ]),
        Line::from(vec![
            Span::styled("● ", Style::default().fg(Color::LightGreen)),
            Span::styled("instant verifier", Style::default().fg(Color::Gray)),
        ]),
        Line::from(vec![
            Span::styled("● ", Style::default().fg(Color::LightGreen)),
            Span::styled("local docs + Vim dojo", Style::default().fg(Color::Gray)),
        ]),
    ];
    if transition.is_some() {
        mission.push(Line::from(""));
        mission.push(Line::from(Span::styled(
            format!("launch sequence  {progress}%"),
            Style::default().fg(Color::Yellow),
        )));
        mission.push(start_progress_line(progress, 100, mission_width.min(22)));
    }
    let mission = Paragraph::new(mission)
        .style(Style::default().bg(Color::Rgb(15, 24, 34)))
        .block(
            Block::default()
                .title(" ◌  RUN STATUS ")
                .padding(Padding::uniform(1)),
        )
        .wrap(Wrap { trim: true });
    frame.render_widget(mission, columns[1]);

    let footer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(3)])
        .split(rows[2]);
    let journey = Paragraph::new(Line::from(vec![
        Span::styled("  JOURNEY  ", Style::default().fg(Color::DarkGray)),
        Span::styled("●", Style::default().fg(Color::LightCyan)),
        Span::styled("━━", Style::default().fg(Color::Cyan)),
        Span::styled("○", Style::default().fg(Color::DarkGray)),
        Span::styled("━━", Style::default().fg(Color::DarkGray)),
        Span::styled("○", Style::default().fg(Color::DarkGray)),
        Span::styled("━━", Style::default().fg(Color::DarkGray)),
        Span::styled("○", Style::default().fg(Color::DarkGray)),
        Span::styled("━━", Style::default().fg(Color::DarkGray)),
        Span::styled("○", Style::default().fg(Color::DarkGray)),
        Span::styled(
            "  foundation → files → search → shellcraft",
            Style::default().fg(Color::Gray),
        ),
    ]))
    .block(Block::default().padding(Padding::top(1)));
    frame.render_widget(journey, footer[0]);
    let footer_columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(footer[1]);
    let shortcuts = Paragraph::new(Line::from(vec![
        Span::styled(" Ctrl-Space ", keycap_style()),
        Span::styled(" command palette    ", Style::default().fg(Color::Gray)),
        Span::styled(" calm ", keycap_style()),
        Span::styled(" motion", Style::default().fg(Color::Gray)),
    ]))
    .block(Block::default().padding(Padding::top(1)));
    frame.render_widget(shortcuts, footer_columns[0]);
    let status = Paragraph::new(Line::from(vec![Span::styled(
        if transition.is_some() {
            "  preparing your private shell…"
        } else {
            "  ready when you are  ›"
        },
        Style::default().fg(if transition.is_some() {
            Color::Yellow
        } else {
            Color::LightGreen
        }),
    )]))
    .alignment(Alignment::Right)
    .block(Block::default().padding(Padding::top(1)));
    frame.render_widget(status, footer_columns[1]);
}

fn render_compact_start_screen(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    transition: Option<Duration>,
    challenge_count: usize,
) {
    let progress = start_transition_percent(transition);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(7),
            Constraint::Length(3),
        ])
        .split(area);
    let header = Paragraph::new(Line::from(vec![
        Span::styled(" LEARN", Style::default().fg(Color::LightCyan).bold()),
        Span::styled(" / LINUX DOJO", Style::default().fg(Color::Gray)),
        Span::styled("  •  OFFLINE", Style::default().fg(Color::DarkGray)),
    ]))
    .block(Block::default().padding(Padding::bottom(1)));
    frame.render_widget(header, rows[0]);
    let action = if transition.is_some() {
        format!("OPENING FIRST QUEST  [{progress:02}%]")
    } else {
        "[ ENTER ]  START LEARNING".to_owned()
    };
    let body = Paragraph::new(vec![
        Line::from(Span::styled(
            "L E A R N   L I N U X",
            Style::default().fg(Color::LightCyan).bold(),
        )),
        Line::from("keyboard-first shell dojo"),
        Line::from(""),
        Line::from(Span::styled(
            format!("{challenge_count} quests  •  virtual Linux  •  no mouse required"),
            Style::default().fg(Color::Yellow),
        )),
        Line::from(""),
        Line::from(Span::styled(
            action,
            Style::default()
                .fg(Color::Black)
                .bg(Color::LightGreen)
                .bold(),
        )),
    ])
    .alignment(Alignment::Center)
    .style(Style::default().bg(Color::Rgb(13, 21, 31)))
    .block(
        Block::default()
            .title(" ◈ WELCOME ")
            .padding(Padding::uniform(1)),
    );
    frame.render_widget(body, rows[1]);
    let footer = Paragraph::new("Ctrl-Space palette  •  Ctrl-T themes  •  Esc quit")
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(footer, rows[2]);
}

fn render_start_backdrop(frame: &mut ratatui::Frame<'_>, area: Rect, time: Duration) {
    let points = [
        (3_u16, 3_u16),
        (12, 8),
        (24, 4),
        (37, 12),
        (52, 5),
        (68, 9),
        (83, 3),
        (97, 14),
        (112, 7),
        (127, 4),
        (141, 12),
        (154, 6),
    ];
    let phase = time.as_millis() / 600;
    let buffer = frame.buffer_mut();
    for (index, (x, y)) in points.into_iter().enumerate() {
        let x = area.x.saturating_add(x);
        let y = area.y.saturating_add(y);
        if x >= area.right() || y >= area.bottom() {
            continue;
        }
        let brightness = (phase + u128::try_from(index).unwrap_or(0)) % 4;
        let color = match brightness {
            0 => Color::Rgb(24, 49, 61),
            1 => Color::Rgb(35, 70, 82),
            _ => Color::Rgb(26, 54, 68),
        };
        if let Some(cell) = buffer.cell_mut((x, y)) {
            cell.set_symbol(if brightness == 1 { "✦" } else { "·" })
                .set_style(Style::default().fg(color));
        }
    }
}

fn start_transition_percent(transition: Option<Duration>) -> usize {
    transition.map_or(0, |value| {
        usize::try_from((value.as_millis().min(650) * 100) / 650).unwrap_or(100)
    })
}

fn block_glyph(character: char) -> [&'static str; 5] {
    match character {
        'L' => ["██   ", "██   ", "██   ", "██   ", "█████"],
        'E' => ["█████", "██   ", "████  ", "██   ", "█████"],
        'A' => [" ███ ", "██ ██", "█████", "██ ██", "██ ██"],
        'R' => ["████ ", "██ ██", "████ ", "██ ██", "██ ██"],
        'N' => ["██ ██", "█████", "█████", "██ ██", "██ ██"],
        'I' => ["█████", "  ██ ", "  ██ ", "  ██ ", "█████"],
        'U' => ["██ ██", "██ ██", "██ ██", "██ ██", " ███ "],
        'X' => ["██ ██", "██ ██", " ███ ", "██ ██", "██ ██"],
        _ => ["     "; 5],
    }
}

fn start_word_lines(word: &str, style: Style) -> Vec<Line<'static>> {
    (0..5)
        .map(|row| {
            let mut spans = vec![Span::raw("  ")];
            for character in word.chars() {
                spans.push(Span::styled(block_glyph(character)[row], style));
                spans.push(Span::raw(" "));
            }
            Line::from(spans)
        })
        .collect()
}

fn start_progress_line(current: usize, total: usize, width: usize) -> Line<'static> {
    let filled = width.saturating_mul(current.min(total)) / total.max(1);
    Line::from(vec![
        Span::styled("[", Style::default().fg(Color::DarkGray)),
        Span::styled("━".repeat(filled), Style::default().fg(Color::LightGreen)),
        Span::styled(
            "·".repeat(width.saturating_sub(filled)),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled("]", Style::default().fg(Color::DarkGray)),
    ])
}

fn keycap_style() -> Style {
    Style::default()
        .fg(Color::Black)
        .bg(Color::Rgb(135, 192, 190))
        .add_modifier(Modifier::BOLD)
}

fn challenge_lines(
    daemon: &Daemon,
    selected_index: usize,
    width: usize,
    theme: Theme,
) -> Vec<Line<'static>> {
    let item_width = width.saturating_sub(5).max(8);
    let mut lines = vec![
        Line::from(Span::styled(
            "QUEST SELECT",
            Style::default().fg(theme.warn).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "Choose a mission. The shell is waiting.",
            theme.muted(),
        )),
        Line::from(""),
    ];
    for (index, challenge) in daemon.content.challenges.iter().enumerate() {
        let selected = index == selected_index;
        let marker = if selected { "▸" } else { " " };
        let style = if selected {
            theme.selected()
        } else {
            theme.dim()
        };
        lines.push(Line::from(Span::styled(
            format!(
                "{marker} {:02} {}",
                index + 1,
                truncate_text(&challenge.spec.meta.title, item_width)
            ),
            style,
        )));
        lines.push(Line::from(Span::styled(
            format!(
                "    {}",
                truncate_text(challenge.spec.id.as_str(), item_width)
            ),
            Style::default().fg(if selected { theme.accent } else { theme.muted }),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "↑/↓ choose • Enter open • Esc close",
        theme.muted(),
    )));
    lines
}

fn brief_height(rows: u16) -> u16 {
    if rows >= 28 {
        12
    } else if rows >= 20 {
        10
    } else {
        9
    }
}

fn hint_click_index(column: u16, row: u16, columns: u16, rows: u16) -> Option<usize> {
    let right = if columns >= 118 {
        (columns.saturating_mul(28) / 100).max(52)
    } else {
        columns.saturating_mul(28) / 100
    };
    let left = columns.saturating_sub(right);
    let index = row.checked_sub(4)? as usize;
    (column > 1
        && column < left.saturating_sub(1)
        && index < 3
        && row < brief_height(rows).saturating_sub(3))
    .then_some(index)
}

fn masked_hint(
    text: &str,
    revealed: Option<Duration>,
    now: Duration,
    animations_enabled: bool,
    width: usize,
) -> String {
    let visible = if animations_enabled {
        revealed.map_or(0, |at| {
            ((now.saturating_sub(at).as_millis() as usize * width) / 330).min(width)
        })
    } else if revealed.is_some() {
        width
    } else {
        0
    };
    text.chars()
        .take(width)
        .enumerate()
        .map(|(index, character)| {
            if index < visible {
                character
            } else if character.is_whitespace() {
                ' '
            } else {
                '▒'
            }
        })
        .collect()
}

fn render_locked_hints(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    hints: &[learn_challenge::Hint],
    revealed: &[Option<Duration>; 3],
    now: Duration,
    animations_enabled: bool,
    theme: Theme,
) {
    let available = area.height.saturating_sub(5).min(3) as usize;
    let text_width = usize::from(area.width.saturating_sub(5));
    for (index, hint) in hints.iter().take(available).enumerate() {
        let marker = format!("[{}] ", index + 1);
        let text = masked_hint(
            &hint.text,
            revealed[index],
            now,
            animations_enabled,
            text_width,
        );
        let style = if revealed[index].is_some() {
            theme.accent()
        } else {
            theme.dim()
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(marker, style),
                Span::styled(
                    text,
                    if revealed[index].is_some() {
                        Style::default().fg(theme.fg)
                    } else {
                        theme.muted()
                    },
                ),
            ])),
            Rect::new(
                area.x,
                area.y.saturating_add(3 + index as u16),
                area.width,
                1,
            ),
        );
    }
}

fn render_level_progress(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    selected_index: usize,
    total: usize,
    started: Option<Duration>,
    now: Duration,
    animations_enabled: bool,
    theme: Theme,
) {
    if area.width == 0 || area.height < 2 || total == 0 {
        return;
    }
    let completed = selected_index.min(total);
    let label_y = area.bottom().saturating_sub(2);
    let current = selected_index.saturating_add(1).min(total);
    let label = Line::from(vec![
        Span::styled(
            format!("LEVEL {current:02}  ",),
            theme.accent().add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("{completed} of {total} complete"), theme.dim()),
    ]);
    frame.render_widget(
        Paragraph::new(label),
        Rect::new(area.x, label_y, area.width, 1),
    );
    let motion = if animations_enabled {
        started.map(|start| (now.saturating_sub(start).as_secs_f64() / 0.72).clamp(0.0, 1.0))
    } else {
        None
    };
    let eased = motion.map_or(1.0, |phase| 1.0 - (1.0 - phase).powi(3));
    let displayed = if motion.is_some() && completed > 0 {
        (completed - 1) as f64 + eased
    } else {
        completed as f64
    };
    let ratio = displayed / total as f64;
    let bar = Rect::new(area.x, label_y.saturating_add(1), area.width, 1);
    frame.render_widget(
        Gauge::default()
            .gauge_style(Style::default().fg(theme.success).bg(theme.overlay))
            .ratio(ratio)
            .label(""),
        bar,
    );
    let filled = (ratio * f64::from(bar.width)).round() as u16;
    let buffer = frame.buffer_mut();
    for column in 0..filled.min(bar.width) {
        if let Some(cell) = buffer.cell_mut((bar.x.saturating_add(column), bar.y)) {
            let from_tip = filled.saturating_sub(column);
            let color = if motion.is_some_and(|phase| phase < 1.0) && from_tip <= 2 {
                theme.warn
            } else if from_tip <= 5 {
                theme.accent
            } else {
                theme.success
            };
            cell.set_bg(color);
        }
    }
}

fn challenge_brief_lines(
    challenge: Option<&learn_challenge::CompiledChallenge>,
    theme: Theme,
) -> Vec<Line<'static>> {
    let Some(challenge) = challenge else {
        return vec![Line::from(Span::styled("No lesson selected", theme.dim()))];
    };
    vec![
        Line::from(Span::styled(
            challenge.spec.meta.title.clone(),
            Style::default().fg(theme.warn).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            challenge.spec.objective.text.clone(),
            theme.fg,
        )),
    ]
}

fn docs_sidebar_lines(
    query: &str,
    results: &[(String, String)],
    theme: Theme,
) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(Span::styled(
            "LOCAL DOCS",
            theme.accent().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled("Search the command reference", theme.dim())),
        Line::from(""),
        Line::from(Span::styled(
            format!("⌕  {query}▌"),
            Style::default().fg(theme.warn),
        )),
        Line::from(""),
    ];
    if results.is_empty() {
        lines.push(Line::from(Span::styled(
            "Ctrl-L focuses this search field.",
            theme.muted(),
        )));
        lines.push(Line::from(Span::styled(
            "Try: pwd, ls, cat, grep, pipes",
            theme.dim(),
        )));
    } else {
        for (title, snippet) in results {
            lines.push(Line::from(Span::styled(title.clone(), theme.accent())));
            lines.push(Line::from(Span::styled(snippet.clone(), theme.dim())));
            lines.push(Line::from(""));
        }
    }
    lines.push(Line::from(Span::styled(
        "Ctrl-H terminal  •  Ctrl-L docs  •  Esc back",
        theme.muted(),
    )));
    lines
}

fn append_terminal_snapshot(
    history: &mut Vec<Line<'static>>,
    parser: &vt100::Parser,
    width: u16,
    theme: Theme,
) {
    let mut snapshot = terminal_lines_with_cursor(parser, width, false, theme);
    while snapshot
        .last()
        .is_some_and(|line| line.spans.iter().all(|span| span.content.trim().is_empty()))
    {
        snapshot.pop();
    }
    let text = |line: &Line<'_>| {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
    };
    if snapshot
        .last()
        .is_some_and(|line| text(line).trim_start().starts_with('❯'))
    {
        snapshot.pop();
        if snapshot
            .last()
            .is_some_and(|line| text(line).contains("learner@learn"))
        {
            snapshot.pop();
        }
    }
    history.extend(snapshot);
}

fn next_challenge_history_lines(
    challenge: &learn_challenge::CompiledChallenge,
    theme: Theme,
) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            format!("Next  ·  {}", challenge.spec.meta.title),
            theme.accent().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            challenge.spec.objective.text.clone(),
            theme.fg,
        )),
        Line::from(""),
    ]
}

fn completion_lines(
    challenge: Option<&learn_challenge::CompiledChallenge>,
    score: usize,
    total: usize,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let title = challenge.map_or("Lesson", |item| item.spec.meta.title.as_str());
    let box_width = usize::from(width).min(48).max(4);
    let content_width = box_width.saturating_sub(3);
    let border = Style::default().fg(theme.success);
    let card_line = |value: String, style: Style| {
        let value = truncate_text(&value, content_width);
        let padding = content_width.saturating_sub(value.chars().count());
        Line::from(vec![
            Span::styled("│ ", border),
            Span::styled(format!("{value}{}", " ".repeat(padding)), style),
            Span::styled("│", border),
        ])
    };
    vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("╭{}╮", "─".repeat(box_width.saturating_sub(2))),
            border,
        )),
        card_line(
            "✓ Completed".to_owned(),
            Style::default()
                .fg(theme.success)
                .add_modifier(Modifier::BOLD),
        ),
        card_line(
            title.to_owned(),
            Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
        ),
        card_line(format!("{score} of {total} finished"), theme.dim()),
        Line::from(Span::styled(
            format!("╰{}╯", "─".repeat(box_width.saturating_sub(2))),
            border,
        )),
        Line::from(""),
    ]
}

fn render_palette_overlay(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    query: &str,
    selected: usize,
    theme: Theme,
) {
    let dim = theme.surface;
    frame.render_widget(Clear, area);
    frame.render_widget(Block::default().style(Style::default().bg(dim)), area);

    let popup = centered_rect(76, 70, area);
    let shadow = Rect::new(
        popup.x.saturating_add(2),
        popup.y.saturating_add(1),
        popup.width,
        popup.height,
    );
    if shadow.x < area.right() && shadow.y < area.bottom() {
        frame.render_widget(
            Block::default().style(Style::default().bg(theme.muted)),
            shadow,
        );
    }
    let palette = Paragraph::new(palette_lines(query, selected, theme))
        .style(Style::default().fg(theme.fg).bg(theme.overlay))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme.accent())
                .title(" ▣  COMMAND CENTER ")
                .title_bottom(" Ctrl-Space close  •  ↑↓ navigate  •  Enter run ")
                .padding(Padding::uniform(2)),
        )
        .wrap(Wrap { trim: true });
    frame.render_widget(palette, popup);
}

fn render_terminal_pet(frame: &mut ratatui::Frame<'_>, area: Rect, pet: &PetState, theme: Theme) {
    let inner = panel_content_area(area);
    if inner.width < PET_WIDTH || inner.height < PET_HEIGHT {
        return;
    }
    let max_x = inner.width.saturating_sub(PET_WIDTH);
    let x = u16::try_from(pet.x.max(0)).unwrap_or(0).min(max_x);
    let origin_x = inner.x.saturating_add(x);
    let origin_y = inner.bottom().saturating_sub(PET_HEIGHT);
    const MASCOT: [&str; 6] = [
        "   ▄████▄    ",
        " ▄████████▄  ",
        "███  ◉  ◉ ███",
        "███   ▾   ███",
        " ▀████████▀  ",
        "   ▀▀  ▀▀    ",
    ];
    let blink = pet.animation_frame(8) == 7;
    let buffer = frame.buffer_mut();
    for (row, sprite_row) in MASCOT.iter().enumerate() {
        for (column, glyph) in sprite_row.chars().enumerate() {
            if glyph == ' ' {
                continue;
            }
            let x = origin_x.saturating_add(u16::try_from(column).unwrap_or(0));
            let y = origin_y.saturating_add(u16::try_from(row).unwrap_or(0));
            let Some(cell) = buffer.cell_mut((x, y)) else {
                continue;
            };
            if cell.symbol() != " " {
                continue;
            }
            let symbol = if blink && glyph == '◉' {
                "─"
            } else {
                match glyph {
                    '▄' => "▄",
                    '▀' => "▀",
                    '█' => "█",
                    '◉' => "◉",
                    '▾' => "▾",
                    _ => continue,
                }
            };
            cell.set_symbol(symbol)
                .set_fg(if glyph == '◉' || glyph == '▾' {
                    theme.fg
                } else {
                    theme.accent
                });
        }
    }
}

fn render_content_rain(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    challenge: Option<&learn_challenge::CompiledChallenge>,
    time: Duration,
    theme: Theme,
) {
    let inner = panel_content_area(area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let challenge_id = challenge.map_or("ambient", |item| item.spec.id.as_str());
    let seed = challenge_id
        .bytes()
        .enumerate()
        .fold(7_u128, |sum, (index, byte)| {
            sum.wrapping_add(u128::from(byte) * u128::try_from(index + 1).unwrap_or(1))
        });
    let symbols: &[&str] = if challenge_id.contains("search") {
        &["⠂", "╎", "⋮", "┊", "│"]
    } else if challenge_id.contains("files") {
        &["╎", "┆", "┊", "╿", "│"]
    } else {
        &["·", "⠐", "╎", "⋮", "│"]
    };
    let symbol_count = u128::try_from(symbols.len()).unwrap_or(1);
    let tick = time.as_millis() / 120;
    let stream_count = (inner.width / 3).saturating_add(1);
    let buffer = frame.buffer_mut();
    for stream in 0..stream_count {
        let index = u128::from(stream);
        let phase = (seed + index * 41) % u128::from(inner.height.saturating_add(8));
        let fall_rate = 1 + (seed + index * 7) % 3;
        let length = 2 + (seed + index * 13) % 5;
        let span = u128::from(inner.height) + length + 2;
        let head = i32::try_from((tick * fall_rate + phase) % span).unwrap_or(0)
            - i32::try_from(length).unwrap_or(0);
        let base_x = (index * 3 + seed) % u128::from(inner.width);
        let drift = i32::try_from((tick / (fall_rate + 1) + phase) % 5).unwrap_or(0) - 2;
        let x_offset =
            (i32::try_from(base_x).unwrap_or(0) + drift).rem_euclid(i32::from(inner.width));
        let x = inner.x + u16::try_from(x_offset).unwrap_or(0);
        for trail in 0..length {
            let y_offset = head - i32::try_from(trail).unwrap_or(0);
            if y_offset < 0 || y_offset >= i32::from(inner.height) {
                continue;
            }
            let y = inner.y + u16::try_from(y_offset).unwrap_or(0);
            let Some(cell) = buffer.cell_mut((x, y)) else {
                continue;
            };
            if cell.symbol() != " " {
                continue;
            }
            let symbol_index = usize::try_from((seed + index + trail) % symbol_count).unwrap_or(0);
            let style = match trail {
                0 => Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
                1 => Style::default().fg(theme.dim),
                _ => Style::default().fg(theme.muted),
            };
            cell.set_symbol(symbols[symbol_index]).set_style(style);
        }
    }
}

/// Content bounds for the terminal pane. Decorations must never write into
/// the pane's border or into the neighboring challenge/docs views.
fn panel_content_area(area: Rect) -> Rect {
    Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(2),
        area.width.saturating_sub(2),
        area.height.saturating_sub(3),
    )
}

fn truncate_text(value: &str, width: usize) -> String {
    let mut chars = value.chars();
    let text = chars.by_ref().take(width).collect::<String>();
    if chars.next().is_some() && width > 1 {
        format!("{}…", text.chars().take(width - 1).collect::<String>())
    } else {
        text
    }
}

fn palette_lines(query: &str, selected: usize, theme: Theme) -> Vec<Line<'static>> {
    let matches = palette_matches(query);
    let mut lines = vec![Line::from(Span::styled(
        format!("  > {query}▌"),
        theme.title(),
    ))];
    lines.push(Line::from(""));
    if matches.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No matching command.",
            theme.error(),
        )));
    } else {
        for (index, entry_index) in matches.into_iter().enumerate() {
            let active = index == selected;
            let marker = if active { "  ❯ " } else { "    " };
            let style = if active {
                theme.selected()
            } else {
                theme.dim()
            };
            lines.push(Line::from(Span::styled(
                format!("{marker}{}", PALETTE_ENTRIES[entry_index].label),
                style,
            )));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Type to filter • ↑/↓ choose • Enter run",
        theme.muted(),
    )));
    lines
}

fn centered_rect(width_percent: u16, height_percent: u16, area: Rect) -> Rect {
    let requested_width = area.width.saturating_mul(width_percent).saturating_div(100);
    let requested_height = area
        .height
        .saturating_mul(height_percent)
        .saturating_div(100);
    let width = bounded_dimension(requested_width, 44, area.width.saturating_sub(2));
    let height = bounded_dimension(requested_height, 8, area.height.saturating_sub(2));
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

fn bounded_dimension(value: u16, minimum: u16, maximum: u16) -> u16 {
    if maximum < minimum {
        maximum
    } else {
        value.clamp(minimum, maximum)
    }
}

fn pane_size(columns: u16, rows: u16) -> (u16, u16) {
    (
        (columns.saturating_mul(72) / 100).saturating_sub(2).max(1),
        (rows.saturating_mul(80) / 100).saturating_sub(3).max(1),
    )
}

fn key_bytes(code: KeyCode, modifiers: KeyModifiers) -> Option<Vec<u8>> {
    if modifiers.contains(KeyModifiers::CONTROL) {
        if let KeyCode::Char(character) = code {
            let character = character.to_ascii_lowercase();
            if character.is_ascii_lowercase() {
                return Some(vec![character as u8 - b'a' + 1]);
            }
        }
    }
    Some(match code {
        KeyCode::Char(character) => character.to_string().into_bytes(),
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        _ => return None,
    })
}

/// TUI failure.
#[derive(Debug, Error)]
pub enum TuiError {
    /// Terminal I/O failure.
    #[error("terminal I/O failed: {0}")]
    Io(#[source] io::Error),
    /// Runtime failure reserved for the interactive session pane.
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    /// Daemon client failure.
    #[error(transparent)]
    Client(#[from] ClientError),
    /// Local daemon state failure.
    #[error(transparent)]
    Daemon(#[from] learn_daemon::DaemonError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_size_matches_split_terminal_content() {
        assert_eq!(pane_size(100, 40), (70, 29));
        assert_eq!(pane_size(1, 1), (1, 1));
    }

    #[test]
    fn terminal_overlays_stay_inside_their_panel() {
        let panel = Rect::new(4, 5, 80, 30);
        let content = panel_content_area(panel);

        assert_eq!(content, Rect::new(5, 7, 78, 27));
        assert_eq!(content.right(), panel.right().saturating_sub(1));
        assert_eq!(content.bottom(), panel.bottom().saturating_sub(1));
    }

    #[test]
    fn lesson_cursor_is_a_block_and_palette_popup_is_centered() {
        let mut parser = vt100::Parser::new(4, 20, 64);
        parser.process(b"hello");
        let with_cursor = terminal_lines_with_cursor(&parser, 20, true, ThemeId::Default.theme());
        assert_eq!(with_cursor[0].spans.len(), 3);
        assert_eq!(with_cursor[0].spans[1].content, " ");

        let without_cursor =
            terminal_lines_with_cursor(&parser, 20, false, ThemeId::Default.theme());
        assert_eq!(without_cursor[0].spans.len(), 1);

        let popup = centered_rect(72, 55, Rect::new(0, 0, 80, 24));
        assert_eq!(popup, Rect::new(11, 5, 57, 13));
        assert_eq!(centered_rect(72, 55, Rect::new(0, 0, 1, 1)).width, 0);
    }

    #[test]
    fn terminal_snapshot_does_not_append_unused_screen_rows() {
        let mut parser = vt100::Parser::new(20, 40, 64);
        parser.process(b"finished\r\n");
        let rows = terminal_lines_with_cursor(&parser, 40, false, ThemeId::Default.theme());
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn successful_output_waits_for_prompt_before_advancing() {
        let mut pending = Some(Vec::new());
        let mut hints = 0;
        observe_command_output(b"result\r\n", &mut pending, &mut hints);
        assert!(pending.is_some());
        observe_command_output(b"\x07learner@learn", &mut pending, &mut hints);
        assert!(pending.is_none());
        assert_eq!(hints, 0);
    }

    #[test]
    fn completion_snapshot_keeps_file_output_but_omits_interim_prompt() {
        let mut parser = vt100::Parser::new(12, 60, 64);
        parser.process(b"cat welcome.txt\r\nWelcome to a real shell.\r\nlearner@learn  ~  \xe2\x97\x88\r\n\xe2\x9d\xaf ");
        let mut history = Vec::new();
        append_terminal_snapshot(&mut history, &parser, 60, ThemeId::Default.theme());
        let transcript = history
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(transcript.contains("Welcome to a real shell."));
        assert!(!transcript.contains("learner@learn"));
    }

    #[test]
    fn terminal_history_preserves_completed_session_output() {
        let mut parser = vt100::Parser::new(3, 20, 64);
        parser.process(b"hello");
        let mut history = Vec::new();

        append_terminal_snapshot(&mut history, &parser, 20, ThemeId::Default.theme());

        assert!(
            history
                .iter()
                .any(|line| { line.spans.iter().any(|span| span.content.contains("hello")) })
        );
    }

    #[test]
    fn shell_key_encoding_preserves_normal_and_control_input() {
        assert_eq!(
            key_bytes(KeyCode::Char('x'), KeyModifiers::NONE),
            Some(vec![b'x'])
        );
        assert_eq!(
            key_bytes(KeyCode::Char('c'), KeyModifiers::CONTROL),
            Some(vec![3])
        );
        assert_eq!(
            key_bytes(KeyCode::Enter, KeyModifiers::NONE),
            Some(vec![b'\r'])
        );
    }

    #[test]
    fn wrong_shell_submissions_are_tracked_for_hints() {
        let mut input = String::new();
        for character in "definitely-wrong".chars() {
            assert_eq!(
                track_shell_input(&mut input, KeyCode::Char(character), KeyModifiers::NONE),
                None
            );
        }
        assert_eq!(
            track_shell_input(&mut input, KeyCode::Enter, KeyModifiers::NONE),
            Some("definitely-wrong".to_owned())
        );
        assert!(input.is_empty());
    }

    #[test]
    fn command_palette_searches_without_stealing_shell_keys() {
        assert_eq!(palette_matches("vim"), vec![3]);
        assert_eq!(palette_matches("restart"), vec![4]);
        assert_eq!(palette_matches("ch"), vec![1]);
        assert_eq!(palette_matches("pet"), vec![5]);
        assert_eq!(palette_matches("rain"), vec![6]);
        assert_eq!(palette_matches("reastart"), vec![4]);
        assert_eq!(palette_matches("vmi"), vec![3]);
        assert!(app_leader(KeyCode::Char(' '), KeyModifiers::CONTROL));
        assert!(!app_leader(KeyCode::Char(' '), KeyModifiers::NONE));
    }

    #[test]
    fn ctrl_t_cycles_through_every_theme_and_wraps() {
        assert!(theme_cycle_key(KeyCode::Char('t'), KeyModifiers::CONTROL));
        assert!(theme_cycle_key(KeyCode::Char('T'), KeyModifiers::CONTROL));
        assert!(!theme_cycle_key(KeyCode::Char('t'), KeyModifiers::NONE));
        assert!(!theme_cycle_key(KeyCode::Char('x'), KeyModifiers::CONTROL));
        assert_eq!(ThemeId::ALL.len(), 21);
        assert_eq!(ThemeId::Default.next(), ThemeId::Catppuccin);
        let mut seen = vec![ThemeId::Default];
        let mut current = ThemeId::Default.next();
        while current != ThemeId::Default {
            assert!(!seen.contains(&current), "cycle repeated a theme");
            seen.push(current);
            current = current.next();
        }
        assert_eq!(seen.len(), ThemeId::ALL.len());
    }

    #[test]
    fn vim_style_panel_focus_uses_h_and_l() {
        assert_eq!(
            panel_focus_key(KeyCode::Char('h'), KeyModifiers::CONTROL),
            Some(PanelFocus::Left)
        );
        assert_eq!(
            panel_focus_key(KeyCode::Char('l'), KeyModifiers::CONTROL),
            Some(PanelFocus::Right)
        );
        assert_eq!(
            panel_focus_key(KeyCode::Char('r'), KeyModifiers::CONTROL),
            Some(PanelFocus::Right)
        );
        assert_eq!(
            panel_focus_key(KeyCode::Char('h'), KeyModifiers::NONE),
            None
        );
    }

    #[test]
    fn completion_reveal_respects_motion_setting() {
        let envelope = Envelope {
            protocol_version: learn_protocol::PROTOCOL_VERSION,
            message_id: 1,
            body: Some(envelope::Body::Event(learn_protocol::Event {
                kind: Some(protocol_event::Kind::ChallengeState(
                    learn_protocol::ChallengeStateEvent {
                        challenge_id: "foundation.orientation".to_owned(),
                        state: "completed".to_owned(),
                    },
                )),
            })),
        };
        let mut parser = vt100::Parser::new(4, 12, 64);
        let mut completed = false;
        let mut effects = EffectManager::default();

        process_event(
            &envelope,
            &mut parser,
            &mut completed,
            true,
            &mut effects,
            ThemeId::Default.theme(),
        );

        assert!(completed);
        assert!(effects.is_running());

        let mut disabled_effects = EffectManager::default();
        let mut disabled_completed = false;
        process_event(
            &envelope,
            &mut parser,
            &mut disabled_completed,
            false,
            &mut disabled_effects,
            ThemeId::Default.theme(),
        );

        assert!(disabled_completed);
        assert!(!disabled_effects.is_running());
    }
}
