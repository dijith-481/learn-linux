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
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use learn_daemon::{ClientError, Daemon, DaemonClient};
use learn_protocol::{Envelope, envelope, event as protocol_event};
use learn_runtime::{RuntimeError, start_wasm_session};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Padding, Paragraph, Wrap},
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

const PALETTE_ENTRIES: [PaletteEntry; 15] = [
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
    if let Some(best) = matches.iter().map(|(score, _)| *score).min() {
        matches.retain(|(score, _)| *score == best);
    }
    matches.into_iter().map(|(_, index)| index).collect()
}

fn palette_match_score(query: &str, entry: &PaletteEntry) -> Option<usize> {
    if query.is_empty() {
        return Some(usize::MAX);
    }
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
    haystack.contains(query).then_some(3)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PanelFocus {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy)]
struct PetState {
    x: i16,
    y: i16,
    dx: i16,
    dy: i16,
    step_elapsed: Duration,
    mood: u8,
}

const PET_WIDTH: u16 = 16;
const PET_HEIGHT: u16 = 7;

impl Default for PetState {
    fn default() -> Self {
        Self {
            x: 8,
            y: 5,
            dx: 1,
            dy: 0,
            step_elapsed: Duration::ZERO,
            mood: 0,
        }
    }
}

impl PetState {
    fn tick(&mut self, elapsed: Duration, width: u16, height: u16) {
        self.step_elapsed += elapsed;
        if self.step_elapsed < Duration::from_millis(850) {
            return;
        }
        self.step_elapsed = Duration::ZERO;
        self.mood = self.mood.wrapping_add(1);
        // A cat pauses between steps instead of gliding like a screensaver.
        if self.mood % 4 == 0 {
            return;
        }
        let max_x = i16::try_from(width.saturating_sub(PET_WIDTH)).unwrap_or(i16::MAX);
        let max_y = i16::try_from(height.saturating_sub(PET_HEIGHT)).unwrap_or(i16::MAX);
        self.x = self.x.clamp(0, max_x.max(0));
        self.y = self.y.clamp(0, max_y.max(0));
        self.x = self.x.saturating_add(self.dx);
        self.y = self.y.saturating_add(self.dy);
        if self.x <= 0 || self.x >= max_x {
            self.dx = -self.dx;
        }
        if self.y <= 0 || self.y >= max_y {
            self.dy = -self.dy;
        }
        if self.mood % 8 == 0 && self.dx != 0 {
            self.dy = i16::from(self.dy == 0);
        }
    }

    fn handle_key(&mut self, code: KeyCode) -> bool {
        let (dx, dy) = match code {
            KeyCode::Left => (-1, 0),
            KeyCode::Right => (1, 0),
            KeyCode::Up => (0, -1),
            KeyCode::Down => (0, 1),
            _ => return false,
        };
        self.dx = dx;
        self.dy = dy;
        true
    }
}

fn app_leader(code: KeyCode, modifiers: KeyModifiers) -> bool {
    code == KeyCode::Char(' ') && modifiers.contains(KeyModifiers::CONTROL)
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
    execute!(stdout, EnterAlternateScreen, cursor::Hide).map_err(TuiError::Io)?;
    Terminal::new(CrosstermBackend::new(stdout)).map_err(TuiError::Io)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<(), TuiError> {
    disable_raw_mode().map_err(TuiError::Io)?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, cursor::Show).map_err(TuiError::Io)?;
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
    let mut completed = false;
    let mut completion_elapsed = Duration::ZERO;
    let mut score = 0_usize;
    let mut hint_level = 0_usize;
    let mut shell_input = String::new();
    let mut animations_enabled = true;
    let mut mode = AppMode::Start;
    let mut start_transition = None;
    let mut focus = PanelFocus::Left;
    // The pet is an optional atmosphere feature; keep the default lesson
    // focused and let learners enable it from the command palette.
    let mut pet_enabled = false;
    let mut pet = PetState::default();
    let mut rain_enabled = false;
    let mut palette_index = 0_usize;
    let mut palette_query = String::new();
    let mut theme_id = ThemeId::Default;
    let mut vim_dojo = VimDojo::default();
    let mut docs_query = String::new();
    let mut docs_results = Vec::new();
    let mut progress_rows = Vec::new();
    loop {
        let elapsed = last_frame.elapsed();
        last_frame = Instant::now();
        animation_time += elapsed;
        if completed {
            completion_elapsed += elapsed;
        }
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
            pet.tick(elapsed, pane_columns, pane_rows);
        }
        while let Ok(data) = output_receiver.try_recv() {
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
                    completion_elapsed = Duration::ZERO;
                    score = score.saturating_add(1);
                }
            }
        }
        if completed && completion_elapsed >= Duration::from_secs(2) {
            advance_to_next_challenge(
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
                &mut completion_elapsed,
            )
            .await?;
            focus = PanelFocus::Left;
            hint_level = 0;
            shell_input.clear();
        }
        draw_ui(
            terminal,
            daemon,
            selected_challenge.as_ref(),
            selected_index,
            session.is_some() || rpc_session_id.is_some(),
            completed,
            mode,
            start_transition,
            hint_level,
            palette_index,
            &palette_query,
            &docs_query,
            &docs_results,
            &progress_rows,
            &parser,
            focus,
            pet_enabled,
            &pet,
            rain_enabled,
            completion_elapsed,
            score,
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
                                                hint_level = 0;
                                                shell_input.clear();
                                                completion_elapsed = Duration::ZERO;
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
                            KeyCode::Esc => focus = PanelFocus::Left,
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
                                hint_level = 0;
                                shell_input.clear();
                                completion_elapsed = Duration::ZERO;
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
                                        daemon.complete_challenge(&challenge.spec.id)?;
                                    }
                                    completed = true;
                                    completion_elapsed = Duration::ZERO;
                                    score = score.saturating_add(1);
                                    if animations_enabled {
                                        effects.add_effect(fx::fade_from_fg(
                                            theme_id.theme().accent,
                                            (500, Interpolation::QuadOut),
                                        ));
                                    }
                                } else if submitted_command
                                    .as_ref()
                                    .is_some_and(|command| !command.trim().is_empty())
                                    && !completed
                                {
                                    hint_level = hint_level.saturating_add(1);
                                }
                            }
                        }
                        _ => {}
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
    completion_elapsed: &mut Duration,
) -> Result<(), TuiError> {
    let next_index = selected_index.saturating_add(1);
    let Some(challenge) = daemon.content.challenges.get(next_index).cloned() else {
        *completed = false;
        *completion_elapsed = Duration::ZERO;
        return Ok(());
    };
    *selected_index = next_index;
    *selected_challenge = Some(challenge.clone());
    *completed = false;
    *completion_elapsed = Duration::ZERO;
    if let Some(client) = client {
        if let Some(session_id) = rpc_session_id.take() {
            client.close_session(&session_id).await?;
        }
        let opened = client
            .open_session(challenge.spec.id.as_str(), pane_columns, pane_rows)
            .await?;
        *rpc_session_id = Some(opened.session_id);
    } else {
        *session = Some(start_wasm_session(
            challenge.spec.id.as_str(),
            &challenge.spec.setup,
            &challenge.spec.environment,
            pane_columns,
            pane_rows,
        )?);
        if let Some(session) = session.as_mut() {
            spawn_output_reader(session, output_sender)?;
        }
    }
    *parser = vt100::Parser::new(pane_rows, pane_columns, 1_000);
    Ok(())
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
    session_active: bool,
    completed: bool,
    mode: AppMode,
    start_transition: Option<Duration>,
    hint_level: usize,
    palette_index: usize,
    palette_query: &str,
    docs_query: &str,
    docs_results: &[(String, String)],
    progress_rows: &[String],
    parser: &vt100::Parser,
    focus: PanelFocus,
    pet_enabled: bool,
    pet: &PetState,
    rain_enabled: bool,
    completion_elapsed: Duration,
    score: usize,
    animation_time: Duration,
    elapsed: Duration,
    effects: &mut EffectManager<()>,
    _animations_enabled: bool,
    vim_dojo: &VimDojo,
    theme: Theme,
) -> Result<(), TuiError> {
    terminal
        .draw(|frame| {
            let area = frame.area();
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
            let right_width = if mode == AppMode::Challenges { 36 } else { 28 };
            let columns = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Percentage(100 - right_width),
                    Constraint::Percentage(right_width),
                ])
                .split(area);
            let left_sections = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(20), Constraint::Percentage(80)])
                .split(columns[0]);
            let brief_lines = challenge_brief_lines(selected_challenge, theme);
            let runtime_text = daemon.runtime.render();
            let mut terminal_lines = if mode == AppMode::Vim {
                vim_dojo.main_lines(left_sections[1].width.saturating_sub(2), theme)
            } else if session_active {
                let width = left_sections[1].width.saturating_sub(2);
                terminal_lines_with_cursor(parser, width, mode == AppMode::Lesson, theme)
            } else {
                runtime_text.lines().map(Line::from).collect::<Vec<_>>()
            };
            if mode == AppMode::Lesson && completed {
                let next_title = daemon
                    .content
                    .challenges
                    .get(selected_index.saturating_add(1))
                    .map(|challenge| challenge.spec.meta.title.as_str());
                terminal_lines.extend(completion_lines(
                    selected_challenge,
                    next_title,
                    completion_elapsed,
                    score,
                    theme,
                ));
            }
            let brief_block = Block::default()
                .borders(Borders::ALL)
                .border_style(if focus == PanelFocus::Left {
                    theme.accent()
                } else {
                    theme.muted()
                })
                .title(" QUEST BRIEF ")
                .padding(Padding::uniform(1));
            let brief = Paragraph::new(brief_lines)
                .block(brief_block)
                .wrap(Wrap { trim: true });
            frame.render_widget(brief, left_sections[0]);
            let terminal_block = Block::default()
                .borders(Borders::ALL)
                .border_style(if focus == PanelFocus::Left {
                    theme.accent()
                } else {
                    theme.muted()
                })
                .title(" TERMINAL ")
                .padding(Padding::top(1));
            let left = Paragraph::new(terminal_lines)
                .block(terminal_block)
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
                render_terminal_pet(frame, left_sections[1], animation_time, pet, theme);
            }
            if mode == AppMode::Lesson && hint_level > 0 {
                let hint = selected_challenge.and_then(|challenge| {
                    challenge
                        .spec
                        .hints
                        .get(
                            hint_level
                                .saturating_sub(1)
                                .min(challenge.spec.hints.len().saturating_sub(1)),
                        )
                        .map(|hint| hint.text.as_str())
                });
                render_inline_hint(frame, left_sections[1], hint, theme);
            }
            let right_lines = match mode {
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
            let right_block = Block::default()
                .borders(Borders::ALL)
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
            if mode == AppMode::Palette {
                render_palette_overlay(frame, area, palette_query, palette_index, theme);
            }
            effects.process_effects(elapsed.into(), frame.buffer_mut(), columns[1]);
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
    screen
        .rows(0, width)
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
    if let Some(rest) = contents.strip_prefix("🐧  ") {
        let separator = rest.find("  /  ")?;
        return Some(Line::from(vec![
            Span::styled("🐧  ", Style::default().fg(theme.warn)),
            Span::styled(
                rest[..separator].to_owned(),
                theme.accent().add_modifier(Modifier::BOLD),
            ),
            Span::styled("  /  ", theme.muted()),
            Span::styled(rest[separator + 5..].to_owned(), theme.dim()),
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
    if contents.starts_with("╭─ ") {
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
        && contents.starts_with("╰─")
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
    let footer = Paragraph::new("Ctrl-Space palette  •  Vim dojo in palette  •  Esc quit")
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

fn challenge_brief_lines(
    challenge: Option<&learn_challenge::CompiledChallenge>,
    theme: Theme,
) -> Vec<Line<'static>> {
    let Some(challenge) = challenge else {
        return vec![Line::from(Span::styled("No active quest", theme.dim()))];
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
        Line::from(Span::styled(
            format!("skill  {}", challenge.spec.id),
            theme.muted(),
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
            format!("/ {query}▌"),
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

fn completion_lines(
    challenge: Option<&learn_challenge::CompiledChallenge>,
    next_title: Option<&str>,
    elapsed: Duration,
    score: usize,
    theme: Theme,
) -> Vec<Line<'static>> {
    let xp = usize::try_from((elapsed.as_millis() / 12).min(100)).unwrap_or(100);
    let spinner =
        ["■", "□", "▪", "▫"][usize::try_from((elapsed.as_millis() / 120) % 4).unwrap_or(0)];
    let title = challenge.map_or("Quest complete", |item| item.spec.meta.title.as_str());
    let next_line = next_title.map_or_else(
        || "NEXT QUEST  /  skill path complete".to_owned(),
        |title| format!("NEXT QUEST  /  {title}"),
    );
    vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("{spinner}  QUEST CLEARED  {spinner}"),
            Style::default()
                .fg(theme.success)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            truncate_text(title, 32),
            Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "VERIFIER STATUS  /  ALL OBJECTIVES MATCHED",
            theme.accent(),
        )),
        square_progress_line("XP", xp, 16, theme),
        Line::from(Span::styled(
            format!("score: {score} quests mastered"),
            theme.accent(),
        )),
        Line::from(""),
        Line::from(Span::styled(next_line, theme.dim())),
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

fn square_progress_line(label: &str, value: usize, width: usize, theme: Theme) -> Line<'static> {
    let filled = width.saturating_mul(value.min(100)) / 100;
    Line::from(vec![
        Span::styled(
            format!("{label:>3}  "),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("■".repeat(filled), Style::default().fg(theme.success)),
        Span::styled("·".repeat(width.saturating_sub(filled)), theme.muted()),
        Span::styled(format!("  {value:03}%"), theme.accent()),
    ])
}

fn render_terminal_pet(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    time: Duration,
    pet: &PetState,
    theme: Theme,
) {
    let inner = Rect::new(
        area.x,
        area.y.saturating_add(1),
        area.width,
        area.height.saturating_sub(1),
    );
    if inner.width < PET_WIDTH || inner.height < PET_HEIGHT {
        return;
    }
    let width = PET_WIDTH;
    let height = PET_HEIGHT;
    let max_x = inner.width.saturating_sub(width);
    let max_y = inner.height.saturating_sub(height);
    let x = u16::try_from(pet.x.max(0)).unwrap_or(0).min(max_x);
    let y = u16::try_from(pet.y.max(0)).unwrap_or(0).min(max_y);
    let pet_area = Rect::new(
        inner.x.saturating_add(x),
        inner.y.saturating_add(y),
        width,
        height,
    );
    let blink = (time.as_millis() / 900) % 7 == 0;
    let eyes = if blink { "—  —" } else { "●  ●" };
    let tail = if pet.mood % 2 == 0 { "╱" } else { "╲" };
    let orange = Style::default().fg(theme.pet_primary);
    let orange_light = Style::default().fg(theme.pet_secondary);
    let shadow = Style::default().fg(theme.muted);
    let eyes_style = Style::default().fg(Color::Rgb(48, 39, 35));
    let lines = vec![
        Line::from(Span::styled("  ▗▄██▄▖ ▗▄██▄▖ ", orange_light)),
        Line::from(Span::styled(" ▟██████████▙  ", orange)),
        Line::from(vec![
            Span::styled("▐██  ", orange),
            Span::styled(eyes, eyes_style),
            Span::styled("  ██▌ ", orange),
        ]),
        Line::from(vec![
            Span::styled("▐██   ", orange),
            Span::styled("▾", orange_light),
            Span::styled("   ██▌ ", orange),
        ]),
        Line::from(vec![Span::styled(" ▜██  ᴗ  ██▛ ", orange)]),
        Line::from(vec![
            Span::styled("  ▀███████▀  ", orange),
            Span::styled(tail, orange_light),
        ]),
        Line::from(vec![
            Span::styled("   ▀▀    ▀▀   ", shadow),
            Span::styled(tail, orange_light),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines).alignment(Alignment::Left), pet_area);
}

fn render_inline_hint(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    hint: Option<&str>,
    theme: Theme,
) {
    let Some(hint) = hint else {
        return;
    };
    let height = 3_u16.min(area.height);
    let hint_area = Rect::new(
        area.x.saturating_add(1),
        area.bottom().saturating_sub(height),
        area.width.saturating_sub(2),
        height,
    );
    let lines = vec![Line::from(vec![
        Span::styled("hint  ", theme.accent().add_modifier(Modifier::BOLD)),
        Span::styled(hint, theme.dim()),
    ])];
    frame.render_widget(
        Paragraph::new(lines)
            .style(Style::default().bg(theme.surface))
            .wrap(Wrap { trim: true }),
        hint_area,
    );
}

fn render_content_rain(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    challenge: Option<&learn_challenge::CompiledChallenge>,
    time: Duration,
    theme: Theme,
) {
    let inner = Rect::new(
        area.x,
        area.y.saturating_add(1),
        area.width,
        area.height.saturating_sub(1),
    );
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
        assert!(app_leader(KeyCode::Char(' '), KeyModifiers::CONTROL));
        assert!(!app_leader(KeyCode::Char(' '), KeyModifiers::NONE));
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
