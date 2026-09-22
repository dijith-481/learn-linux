//! Small, opt-in Vim practice games.

use std::time::Duration;

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

use super::theme::Theme;

const SNAKE_WIDTH: i16 = 24;
const SNAKE_HEIGHT: i16 = 10;
const SNAKE_STEP: Duration = Duration::from_millis(220);
const SNAKE_TARGETS: usize = 12;
const REPEAT_TARGET: &str = "I can learn Linux one command at a time.";
const SUBSTITUTE_COMMAND: &str = "%s/hate/love/g";
const WORD_HOP_TARGET: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Menu,
    Snake,
    Repeat,
    TextObject,
    Search,
    Substitute,
    WordHop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditMode {
    Normal,
    Insert,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Point {
    x: i16,
    y: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Left,
    Down,
    Up,
    Right,
}

impl Direction {
    fn delta(self) -> Point {
        match self {
            Self::Left => Point { x: -1, y: 0 },
            Self::Down => Point { x: 0, y: 1 },
            Self::Up => Point { x: 0, y: -1 },
            Self::Right => Point { x: 1, y: 0 },
        }
    }

    fn is_opposite(self, other: Self) -> bool {
        matches!(
            (self, other),
            (Self::Left, Self::Right)
                | (Self::Right, Self::Left)
                | (Self::Up, Self::Down)
                | (Self::Down, Self::Up)
        )
    }
}

#[derive(Debug)]
struct SnakeGame {
    width: i16,
    height: i16,
    body: Vec<Point>,
    direction: Direction,
    apple: Point,
    score: usize,
    accumulator: Duration,
    game_over: bool,
    complete: bool,
    message: String,
}

impl Default for SnakeGame {
    fn default() -> Self {
        Self {
            width: 32,
            height: 14,
            body: vec![
                Point { x: 8, y: 5 },
                Point { x: 7, y: 5 },
                Point { x: 6, y: 5 },
            ],
            direction: Direction::Right,
            apple: Point { x: 14, y: 5 },
            score: 0,
            accumulator: Duration::ZERO,
            game_over: false,
            complete: false,
            message: "Guide the snake with h j k l; collect every Vim key target.".to_owned(),
        }
    }
}

impl SnakeGame {
    fn reset(&mut self) {
        let width = self.width;
        let height = self.height;
        *self = Self::default();
        self.width = width;
        self.height = height;
        self.apple = next_apple(&self.body, self.score, self.width, self.height);
    }

    fn resize(&mut self, width: u16, height: u16) {
        let width = i16::try_from(width.clamp(SNAKE_WIDTH as u16, 96)).unwrap_or(96);
        let height = i16::try_from(height.clamp(SNAKE_HEIGHT as u16, 28)).unwrap_or(28);
        if self.width != width || self.height != height {
            self.width = width;
            self.height = height;
            self.reset();
        }
    }

    fn set_direction(&mut self, direction: Direction) {
        if !direction.is_opposite(self.direction) {
            self.direction = direction;
        }
    }

    fn tick(&mut self, elapsed: Duration) {
        if self.game_over {
            return;
        }
        self.accumulator += elapsed;
        while self.accumulator >= SNAKE_STEP {
            self.accumulator -= SNAKE_STEP;
            self.step();
            if self.game_over {
                break;
            }
        }
    }

    fn step(&mut self) {
        let head = self.body[0];
        let delta = self.direction.delta();
        let next = Point {
            x: head.x + delta.x,
            y: head.y + delta.y,
        };
        if next.x < 0
            || next.x >= self.width
            || next.y < 0
            || next.y >= self.height
            || self.body.contains(&next)
        {
            self.game_over = true;
            self.message = String::from(
                "Wall met snake. Wall won. Press r to retry; Vim cannot edit geometry.",
            );
            return;
        }

        self.body.insert(0, next);
        if next == self.apple {
            self.score += 1;
            if self.score >= SNAKE_TARGETS {
                self.complete = true;
                self.message = String::from(
                    "Dojo clear. Every key target eaten; the next Vim level is loading.",
                );
            } else {
                self.apple = next_apple(&self.body, self.score, self.width, self.height);
                self.message = format!(
                    "Key target eaten. {} remain; the snake remains employed.",
                    SNAKE_TARGETS - self.score
                );
            }
        } else {
            self.body.pop();
        }
    }
}

fn next_apple(body: &[Point], score: usize, width: i16, height: i16) -> Point {
    let playable_width = width.saturating_sub(2).max(1);
    let playable_height = height.saturating_sub(2).max(1);
    let cells = usize::try_from(playable_width.saturating_mul(playable_height)).unwrap_or(1);
    for offset in 0..cells {
        let index = (score.saturating_mul(37) + offset.saturating_mul(11)) % cells;
        let index = i16::try_from(index).unwrap_or(0);
        let candidate = Point {
            x: 1 + index % playable_width,
            y: 1 + index / playable_width,
        };
        if !body.contains(&candidate) {
            return candidate;
        }
    }
    Point {
        x: width.saturating_sub(2).max(1),
        y: height.saturating_sub(2).max(1),
    }
}

fn target_symbol(score: usize) -> &'static str {
    ["h", "j", "k", "l", "w", "b", "e", "i", "a", "x", "d", ":"][score % SNAKE_TARGETS]
}

#[derive(Debug)]
struct RepeatGame {
    buffer: String,
    cursor: usize,
    mode: EditMode,
    pending_delete: bool,
    message: String,
}

impl Default for RepeatGame {
    fn default() -> Self {
        Self {
            buffer: String::new(),
            cursor: 0,
            mode: EditMode::Normal,
            pending_delete: false,
            message: "Press i, type the line, then Esc. Make one typo and repair it with x."
                .to_owned(),
        }
    }
}

impl RepeatGame {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        match self.mode {
            EditMode::Insert => self.handle_insert_key(code, modifiers),
            EditMode::Normal => self.handle_normal_key(code),
        }
    }

    fn handle_insert_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        match code {
            KeyCode::Esc => {
                self.mode = EditMode::Normal;
                self.pending_delete = false;
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.buffer.remove(self.cursor);
                }
            }
            KeyCode::Char(character)
                if !modifiers.contains(KeyModifiers::CONTROL)
                    && !modifiers.contains(KeyModifiers::ALT) =>
            {
                self.buffer.insert(self.cursor, character);
                self.cursor += character.len_utf8();
            }
            _ => {}
        }
    }

    fn handle_normal_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => {
                self.pending_delete = false;
            }
            KeyCode::Char('i') => {
                self.mode = EditMode::Insert;
                self.pending_delete = false;
            }
            KeyCode::Char('a') => {
                self.cursor = next_boundary(&self.buffer, self.cursor);
                self.mode = EditMode::Insert;
                self.pending_delete = false;
            }
            KeyCode::Char('A') => {
                self.cursor = self.buffer.len();
                self.mode = EditMode::Insert;
                self.pending_delete = false;
            }
            KeyCode::Char('0') => self.cursor = 0,
            KeyCode::Char('$') => self.cursor = self.buffer.len(),
            KeyCode::Char('h') | KeyCode::Left => {
                self.cursor = previous_boundary(&self.buffer, self.cursor);
                self.pending_delete = false;
            }
            KeyCode::Char('l') | KeyCode::Right => {
                self.cursor = next_boundary(&self.buffer, self.cursor);
                self.pending_delete = false;
            }
            KeyCode::Char('x') => {
                if self.cursor < self.buffer.len() {
                    let end = next_boundary(&self.buffer, self.cursor);
                    self.buffer.replace_range(self.cursor..end, "");
                }
                self.pending_delete = false;
            }
            KeyCode::Char('d') if self.pending_delete => {
                self.buffer.clear();
                self.cursor = 0;
                self.pending_delete = false;
                self.message = String::from("dd deleted the line. Efficient, slightly alarming.");
            }
            KeyCode::Char('d') => self.pending_delete = true,
            _ => self.pending_delete = false,
        }
    }

    fn is_complete(&self) -> bool {
        self.buffer == REPEAT_TARGET
    }
}

fn previous_boundary(text: &str, cursor: usize) -> usize {
    text[..cursor]
        .char_indices()
        .next_back()
        .map_or(0, |(index, _)| index)
}

fn next_boundary(text: &str, cursor: usize) -> usize {
    text[cursor..]
        .chars()
        .next()
        .map_or(text.len(), |character| cursor + character.len_utf8())
}

#[derive(Debug)]
struct SubstituteGame {
    text: String,
    command: String,
    command_mode: bool,
    complete: bool,
    message: String,
}

impl Default for SubstituteGame {
    fn default() -> Self {
        Self {
            text: "The old script says hate; the new script says hate.".to_owned(),
            command: String::new(),
            command_mode: false,
            complete: false,
            message: "Press : and enter %s/hate/love/g. One command, every match.".to_owned(),
        }
    }
}

impl SubstituteGame {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        if !self.command_mode {
            if let KeyCode::Char(':') = code {
                self.command_mode = true;
                self.message =
                    String::from("Ex command line active. The colon has entered the chat.");
            }
            return;
        }

        match code {
            KeyCode::Esc => {
                self.command_mode = false;
                self.command.clear();
            }
            KeyCode::Backspace => {
                self.command.pop();
            }
            KeyCode::Enter => self.apply_command(),
            KeyCode::Char(character)
                if !modifiers.contains(KeyModifiers::CONTROL)
                    && !modifiers.contains(KeyModifiers::ALT) =>
            {
                self.command.push(character);
            }
            _ => {}
        }
    }

    fn apply_command(&mut self) {
        self.command_mode = false;
        // Accept both forms so copying the displayed Vim command works even
        // when the user has already entered command-line mode with `:`.
        let command = self.command.strip_prefix(':').unwrap_or(&self.command);
        if command == SUBSTITUTE_COMMAND {
            self.text = self.text.replace("hate", "love");
            self.complete = true;
            self.message = String::from(
                "Solved. Global replacement achieved; the buffer has filed no complaint.",
            );
        } else {
            self.message = format!(
                "Not quite. Vim understood '{}'; your future self understood nothing.",
                self.command
            );
        }
        self.command.clear();
    }
}

#[derive(Debug)]
struct WordHopGame {
    words: [&'static str; 6],
    cursor: usize,
    used_word_motion: bool,
    message: String,
}

impl Default for WordHopGame {
    fn default() -> Self {
        Self {
            words: ["pwd", "ls", "cat", "grep", "find", "chmod"],
            cursor: 0,
            used_word_motion: false,
            message: "Normal mode: use w to hop forward and b to hop back through commands."
                .to_owned(),
        }
    }
}

impl WordHopGame {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn handle_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('w') => {
                self.cursor = (self.cursor + 1).min(self.words.len().saturating_sub(1));
                self.used_word_motion = true;
                self.message = String::from("w hopped to the next word. Vim likes momentum.");
            }
            KeyCode::Char('b') => {
                self.cursor = self.cursor.saturating_sub(1);
                self.used_word_motion = true;
                self.message = String::from("b hopped back. A tactical retreat, not a failure.");
            }
            KeyCode::Char('e') => {
                self.cursor = (self.cursor + 1).min(self.words.len().saturating_sub(1));
                self.used_word_motion = true;
                self.message = String::from("e landed at the end of the next word.");
            }
            _ => {}
        }
    }

    fn is_complete(&self) -> bool {
        self.cursor >= WORD_HOP_TARGET && self.used_word_motion
    }
}

#[derive(Debug)]
struct TextObjectGame {
    stage: usize,
    complete: bool,
    message: String,
}

impl Default for TextObjectGame {
    fn default() -> Self {
        Self {
            stage: 0,
            complete: false,
            message: "Change the quoted text with c i \" — operator plus text object.".to_owned(),
        }
    }
}

impl TextObjectGame {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn handle_key(&mut self, code: KeyCode) {
        if self.complete {
            return;
        }
        let expected = ['c', 'i', '"'];
        if let KeyCode::Char(character) = code {
            if character == expected[self.stage] {
                self.stage += 1;
                if self.stage == expected.len() {
                    self.complete = true;
                    self.message = String::from(
                        "ci\" selected the inner text. The buffer has been changed cleanly.",
                    );
                } else {
                    self.message = format!("Good. Next key: {}", expected[self.stage]);
                }
            } else if character != 'r' {
                self.stage = 0;
                self.message = String::from("Sequence reset. Try c then i then a quote.");
            }
        }
    }
}

#[derive(Debug)]
struct SearchGame {
    query: String,
    command_mode: bool,
    complete: bool,
    message: String,
}

impl Default for SearchGame {
    fn default() -> Self {
        Self {
            query: String::new(),
            command_mode: false,
            complete: false,
            message: "Press /, search for TODO, then Enter. n repeats the search.".to_owned(),
        }
    }
}

impl SearchGame {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        if !self.command_mode {
            match code {
                KeyCode::Char('/') => {
                    self.command_mode = true;
                    self.query.clear();
                    self.message = String::from("Search mode. Type TODO, then press Enter.");
                }
                KeyCode::Char('n') if self.query.eq_ignore_ascii_case("TODO") => {
                    self.complete = true;
                    self.message = String::from("n found the next match. Search muscle acquired.");
                }
                _ => {}
            }
            return;
        }
        match code {
            KeyCode::Esc => {
                self.command_mode = false;
                self.query.clear();
            }
            KeyCode::Backspace => {
                self.query.pop();
            }
            KeyCode::Enter => {
                self.command_mode = false;
                if self.query.eq_ignore_ascii_case("TODO") {
                    self.complete = true;
                    self.message = String::from("Match found. /pattern is now a navigation tool.");
                } else {
                    self.message = format!("No match for '{}'. Try TODO.", self.query);
                }
            }
            KeyCode::Char(character)
                if !modifiers.contains(KeyModifiers::CONTROL)
                    && !modifiers.contains(KeyModifiers::ALT) =>
            {
                self.query.push(character);
            }
            _ => {}
        }
    }
}

/// Opt-in Vim learning activities.
#[derive(Debug)]
pub(crate) struct VimDojo {
    screen: Screen,
    menu_index: usize,
    snake: SnakeGame,
    repeat: RepeatGame,
    text_object: TextObjectGame,
    search: SearchGame,
    substitute: SubstituteGame,
    word_hop: WordHopGame,
    transition_elapsed: Duration,
}

impl Default for VimDojo {
    fn default() -> Self {
        Self {
            screen: Screen::Menu,
            menu_index: 0,
            snake: SnakeGame::default(),
            repeat: RepeatGame::default(),
            text_object: TextObjectGame::default(),
            search: SearchGame::default(),
            substitute: SubstituteGame::default(),
            word_hop: WordHopGame::default(),
            transition_elapsed: Duration::ZERO,
        }
    }
}

impl VimDojo {
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn is_menu(&self) -> bool {
        self.screen == Screen::Menu
    }

    pub(crate) fn is_snake(&self) -> bool {
        self.screen == Screen::Snake
    }

    pub(crate) fn resize_snake(&mut self, width: u16, height: u16) {
        if self.is_snake() {
            self.snake.resize(width, height);
        }
    }

    pub(crate) fn tick(&mut self, elapsed: Duration) {
        if self.screen == Screen::Snake {
            self.snake.tick(elapsed);
        }
        if self.current_complete() {
            self.transition_elapsed += elapsed;
            if self.transition_elapsed >= Duration::from_millis(900) {
                self.advance_level();
            }
        } else {
            self.transition_elapsed = Duration::ZERO;
        }
    }

    pub(crate) fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        match self.screen {
            Screen::Menu => self.handle_menu_key(code),
            Screen::Snake => self.handle_snake_key(code),
            Screen::Repeat => self.handle_repeat_key(code, modifiers),
            Screen::TextObject => self.handle_text_object_key(code),
            Screen::Search => self.handle_search_key(code, modifiers),
            Screen::Substitute => self.handle_substitute_key(code, modifiers),
            Screen::WordHop => self.handle_word_hop_key(code),
        }
    }

    pub(crate) fn main_lines(&self, width: u16, theme: Theme) -> Vec<Line<'static>> {
        match self.screen {
            Screen::Menu => self.menu_lines(width, theme),
            Screen::Snake => self.snake_lines(width, theme),
            Screen::Repeat => self.repeat_lines(width, theme),
            Screen::TextObject => self.text_object_lines(width, theme),
            Screen::Search => self.search_lines(width, theme),
            Screen::Substitute => self.substitute_lines(width, theme),
            Screen::WordHop => self.word_hop_lines(width, theme),
        }
    }

    pub(crate) fn side_lines(&self, theme: Theme) -> Vec<Line<'static>> {
        let mut lines = vec![Line::from(Span::styled(
            "Vim dojo (Esc close)",
            Style::default().fg(theme.warn),
        ))];
        match self.screen {
            Screen::Menu => {
                lines.extend([
                    Line::from("Optional. No Vim knowledge required."),
                    Line::from("Choose a tiny game; learn by doing."),
                    Line::from(""),
                    Line::from("1 Snake — h j k l"),
                    Line::from("2 Repeat & repair — i Esc x"),
                    Line::from("3 Text object — ci\""),
                    Line::from("4 Search — /pattern n"),
                    Line::from("5 Ex substitute — %s/.../.../g"),
                    Line::from("6 Word hop — w b e"),
                    Line::from(""),
                    Line::from("↑/↓ choose • 1–6 quick start • Enter start"),
                ]);
            }
            Screen::Snake => lines.extend([
                Line::from("Snake"),
                Line::from("Move with h j k l."),
                Line::from(format!("Eat all {SNAKE_TARGETS} Vim key targets.")),
                Line::from("r reset • Esc menu"),
                Line::from(""),
                Line::from(self.snake.message.clone()),
            ]),
            Screen::Repeat => lines.extend([
                Line::from("Repeat & repair"),
                Line::from("Normal: i/a/A, h/l, 0/$, x, dd"),
                Line::from("Insert: type • Esc normal"),
                Line::from("Esc again returns to menu"),
                Line::from(""),
                Line::from(self.repeat.message.clone()),
                Line::from(if self.repeat.is_complete() {
                    "Solved. Your fingers have met the buffer."
                } else {
                    "The buffer is judging every keystroke."
                }),
            ]),
            Screen::TextObject => lines.extend([
                Line::from("Text object trainer"),
                Line::from("Use c i \" to change inside quotes."),
                Line::from("Operator + motion = precise editing."),
                Line::from("Esc menu • r reset"),
                Line::from(""),
                Line::from(self.text_object.message.clone()),
            ]),
            Screen::Search => lines.extend([
                Line::from("Search & jump"),
                Line::from("/ starts a search; Enter accepts it; n repeats."),
                Line::from("Esc menu • r reset"),
                Line::from(""),
                Line::from(self.search.message.clone()),
            ]),
            Screen::Substitute => lines.extend([
                Line::from("Ex substitute"),
                Line::from("Press : then type the command."),
                Line::from("Use g so both matches change."),
                Line::from("Esc menu • r is not a command here"),
                Line::from(""),
                Line::from(self.substitute.message.clone()),
                Line::from(if self.substitute.complete {
                    "Solved. That was suspiciously efficient."
                } else {
                    "No shell, no plugin, just a useful edit."
                }),
            ]),
            Screen::WordHop => lines.extend([
                Line::from("Word hop"),
                Line::from("Normal mode: w / b / e move by words."),
                Line::from("Reach chmod, then the dojo returns to its menu."),
                Line::from("Esc menu • r reset"),
                Line::from(""),
                Line::from(self.word_hop.message.clone()),
            ]),
        }
        lines
    }

    pub(crate) fn fullscreen_lines(
        &self,
        width: u16,
        height: u16,
        theme: Theme,
    ) -> Vec<Line<'static>> {
        let board_width = self.snake.width.max(SNAKE_WIDTH);
        let board_height = self.snake.height.max(SNAKE_HEIGHT);
        let mut lines = vec![Line::from(Span::styled(
            format!(
                "S N A K E   //   score {:02}/{SNAKE_TARGETS:02}   //   h j k l navigation",
                self.snake.score
            ),
            theme.accent().add_modifier(Modifier::BOLD),
        ))];
        for y in 0..board_height {
            let mut spans = Vec::new();
            for x in 0..board_width {
                let point = Point { x, y };
                let (symbol, style) = if self.snake.body.first() == Some(&point) {
                    ("◉", theme.selected().add_modifier(Modifier::BOLD))
                } else if self.snake.body.contains(&point) {
                    ("•", Style::default().fg(theme.success).bg(theme.overlay))
                } else if self.snake.apple == point {
                    (
                        target_symbol(self.snake.score),
                        Style::default()
                            .fg(theme.warn)
                            .bg(theme.overlay)
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    (" ", Style::default().bg(theme.surface))
                };
                spans.push(Span::styled(symbol, style));
            }
            lines.push(Line::from(spans));
        }
        lines.push(Line::from(""));
        let status = if self.snake.game_over {
            "WALL MET SNAKE  •  press r to retry  •  Esc menu".to_owned()
        } else if self.snake.complete {
            "DOJO CLEAR  •  next Vim level loading…".to_owned()
        } else {
            self.snake.message.clone()
        };
        lines.push(Line::from(Span::styled(
            status,
            theme.error().add_modifier(if self.snake.game_over {
                Modifier::BOLD
            } else {
                Modifier::empty()
            }),
        )));
        lines.push(Line::from(Span::styled(
            format!("board {width}×{height}  •  eat the glowing keycaps, then advance"),
            theme.muted(),
        )));
        lines
    }

    fn handle_menu_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Up => self.menu_index = self.menu_index.saturating_sub(1),
            KeyCode::Down => self.menu_index = (self.menu_index + 1).min(5),
            KeyCode::Char('1') => {
                self.menu_index = 0;
                self.screen = Screen::Snake;
            }
            KeyCode::Char('2') => {
                self.menu_index = 1;
                self.screen = Screen::Repeat;
            }
            KeyCode::Char('3') => {
                self.menu_index = 2;
                self.screen = Screen::TextObject;
            }
            KeyCode::Char('4') => {
                self.menu_index = 3;
                self.screen = Screen::Search;
            }
            KeyCode::Char('5') => {
                self.menu_index = 4;
                self.screen = Screen::Substitute;
            }
            KeyCode::Char('6') => {
                self.menu_index = 5;
                self.screen = Screen::WordHop;
            }
            KeyCode::Enter => {
                self.screen = match self.menu_index {
                    0 => Screen::Snake,
                    1 => Screen::Repeat,
                    2 => Screen::TextObject,
                    3 => Screen::Search,
                    4 => Screen::Substitute,
                    _ => Screen::WordHop,
                };
            }
            _ => {}
        }
    }

    fn handle_snake_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => self.screen = Screen::Menu,
            KeyCode::Char('r') => self.snake.reset(),
            KeyCode::Char('h') => self.snake.set_direction(Direction::Left),
            KeyCode::Char('j') => self.snake.set_direction(Direction::Down),
            KeyCode::Char('k') => self.snake.set_direction(Direction::Up),
            KeyCode::Char('l') => self.snake.set_direction(Direction::Right),
            _ => {}
        }
    }

    fn handle_repeat_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        if code == KeyCode::Char('r') && self.repeat.mode == EditMode::Normal {
            self.repeat.reset();
            return;
        }
        if code == KeyCode::Esc && self.repeat.mode == EditMode::Normal {
            self.screen = Screen::Menu;
            return;
        }
        self.repeat.handle_key(code, modifiers);
    }

    fn handle_substitute_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        if code == KeyCode::Char('r') && !self.substitute.command_mode {
            self.substitute.reset();
            return;
        }
        if code == KeyCode::Esc && !self.substitute.command_mode {
            self.screen = Screen::Menu;
            return;
        }
        self.substitute.handle_key(code, modifiers);
    }

    fn handle_text_object_key(&mut self, code: KeyCode) {
        if code == KeyCode::Char('r') {
            self.text_object.reset();
            return;
        }
        if code == KeyCode::Esc {
            self.screen = Screen::Menu;
            return;
        }
        self.text_object.handle_key(code);
    }

    fn handle_search_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        if code == KeyCode::Char('r') && !self.search.command_mode {
            self.search.reset();
            return;
        }
        if code == KeyCode::Esc && !self.search.command_mode {
            self.screen = Screen::Menu;
            return;
        }
        self.search.handle_key(code, modifiers);
    }

    fn handle_word_hop_key(&mut self, code: KeyCode) {
        if code == KeyCode::Char('r') {
            self.word_hop.reset();
            return;
        }
        if code == KeyCode::Esc {
            self.screen = Screen::Menu;
            return;
        }
        self.word_hop.handle_key(code);
    }

    fn current_complete(&self) -> bool {
        match self.screen {
            Screen::Menu => false,
            Screen::Snake => self.snake.complete,
            Screen::Repeat => self.repeat.is_complete(),
            Screen::TextObject => self.text_object.complete,
            Screen::Search => self.search.complete,
            Screen::Substitute => self.substitute.complete,
            Screen::WordHop => self.word_hop.is_complete(),
        }
    }

    fn advance_level(&mut self) {
        self.transition_elapsed = Duration::ZERO;
        match self.screen {
            Screen::Snake => {
                self.screen = Screen::Repeat;
                self.repeat.reset();
            }
            Screen::Repeat => {
                self.screen = Screen::TextObject;
                self.text_object.reset();
            }
            Screen::TextObject => {
                self.screen = Screen::Search;
                self.search.reset();
            }
            Screen::Search => {
                self.screen = Screen::Substitute;
                self.substitute.reset();
            }
            Screen::Substitute => {
                self.screen = Screen::WordHop;
                self.word_hop.reset();
            }
            Screen::WordHop => {
                self.screen = Screen::Menu;
                self.menu_index = 0;
            }
            Screen::Menu => {}
        }
    }

    fn menu_lines(&self, width: u16, theme: Theme) -> Vec<Line<'static>> {
        let rule = "─".repeat(width.saturating_sub(4).max(1) as usize);
        vec![
            Line::from(Span::styled(
                "VIM DOJO",
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(rule),
            Line::from("A tiny keyboard gym for people who distrust mice."),
            Line::from(""),
            menu_item(
                self.menu_index == 0,
                "Snake — h j k l and mild panic",
                theme,
            ),
            menu_item(
                self.menu_index == 1,
                "Repeat & repair — edit without flailing",
                theme,
            ),
            menu_item(
                self.menu_index == 2,
                "Text objects — make ci\" precise",
                theme,
            ),
            menu_item(
                self.menu_index == 3,
                "Search — find TODO with / and n",
                theme,
            ),
            menu_item(
                self.menu_index == 4,
                "Ex substitute — make %s useful",
                theme,
            ),
            menu_item(self.menu_index == 5, "Word hop — learn w b e", theme),
            Line::from(""),
            Line::from("↑/↓ choose • 1–6 quick start • Enter play"),
            Line::from("Esc returns to the Linux lesson."),
        ]
    }

    fn snake_lines(&self, _width: u16, theme: Theme) -> Vec<Line<'static>> {
        let mut lines = vec![Line::from(Span::styled(
            format!("SNAKE  score: {}", self.snake.score),
            Style::default().fg(theme.success),
        ))];
        for y in 0..SNAKE_HEIGHT {
            let mut row = String::with_capacity(SNAKE_WIDTH as usize);
            for x in 0..SNAKE_WIDTH {
                let point = Point { x, y };
                let symbol = if self.snake.body.first() == Some(&point) {
                    '@'
                } else if self.snake.body.contains(&point) {
                    'o'
                } else if self.snake.apple == point {
                    target_symbol(self.snake.score)
                        .chars()
                        .next()
                        .unwrap_or('?')
                } else {
                    ' '
                };
                row.push(symbol);
            }
            lines.push(Line::from(row));
        }
        lines.push(Line::from(format!(
            "score {} / {SNAKE_TARGETS} • {}",
            self.snake.score, self.snake.message
        )));
        lines
    }

    fn repeat_lines(&self, _width: u16, theme: Theme) -> Vec<Line<'static>> {
        vec![
            Line::from(Span::styled(
                format!("REPEAT & REPAIR — {:?} mode", self.repeat.mode),
                Style::default().fg(theme.success),
            )),
            Line::from("Target:"),
            Line::from(REPEAT_TARGET),
            Line::from("Buffer:"),
            cursor_line(
                &self.repeat.buffer,
                self.repeat.cursor,
                self.repeat.mode,
                theme,
            ),
            Line::from(""),
            Line::from(self.repeat.message.clone()),
        ]
    }

    fn substitute_lines(&self, _width: u16, theme: Theme) -> Vec<Line<'static>> {
        let command = if self.substitute.command_mode {
            format!(":{}", self.substitute.command)
        } else {
            "Press : to edit with Ex.".to_owned()
        };
        vec![
            Line::from(Span::styled(
                "EX SUBSTITUTE",
                Style::default().fg(theme.success),
            )),
            Line::from("Text buffer:"),
            Line::from(self.substitute.text.clone()),
            Line::from(""),
            Line::from(format!("Command: {command}")),
            Line::from(format!("Try: {SUBSTITUTE_COMMAND}")),
            Line::from(""),
            Line::from(self.substitute.message.clone()),
        ]
    }

    fn text_object_lines(&self, _width: u16, theme: Theme) -> Vec<Line<'static>> {
        let text = if self.text_object.complete {
            "const message = \"changed cleanly\";"
        } else {
            "const message = \"change this text\";"
        };
        vec![
            Line::from(Span::styled(
                "TEXT OBJECTS — OPERATOR + MOTION",
                Style::default()
                    .fg(theme.success)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(text),
            Line::from(""),
            Line::from("Target sequence: c i \""),
            Line::from(self.text_object.message.clone()),
        ]
    }

    fn search_lines(&self, _width: u16, theme: Theme) -> Vec<Line<'static>> {
        let query = if self.search.command_mode {
            format!("/{}▌", self.search.query)
        } else {
            String::from("/TODO")
        };
        vec![
            Line::from(Span::styled(
                "SEARCH & JUMP — / n",
                Style::default()
                    .fg(theme.success)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from("deploy.sh"),
            Line::from("notes.md"),
            Line::from("TODO: replace the temporary path"),
            Line::from("release.txt"),
            Line::from(""),
            Line::from(format!("search: {query}")),
            Line::from(self.search.message.clone()),
        ]
    }

    fn word_hop_lines(&self, _width: u16, theme: Theme) -> Vec<Line<'static>> {
        let words = self
            .word_hop
            .words
            .iter()
            .enumerate()
            .map(|(index, word)| {
                if index == self.word_hop.cursor {
                    Span::styled(
                        format!("[{word}] "),
                        theme.selected().add_modifier(Modifier::BOLD),
                    )
                } else {
                    Span::styled(format!(" {word}  "), theme.dim())
                }
            })
            .collect::<Vec<_>>();
        vec![
            Line::from(Span::styled(
                "WORD HOP — NORMAL MODE",
                Style::default()
                    .fg(theme.success)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from("Move the cursor through a command buffer:"),
            Line::from(words),
            Line::from(""),
            Line::from("w next word   b previous word   e word end"),
            Line::from(self.word_hop.message.clone()),
        ]
    }
}

fn menu_item(selected: bool, label: &str, theme: Theme) -> Line<'static> {
    let marker = if selected { ">" } else { " " };
    Line::from(Span::styled(
        format!("{marker} {label}"),
        if selected {
            theme.selected()
        } else {
            theme.dim()
        },
    ))
}

fn cursor_line(text: &str, cursor: usize, mode: EditMode, theme: Theme) -> Line<'static> {
    let cursor = cursor.min(text.len());
    let mut spans = vec![Span::raw(text[..cursor].to_owned())];
    let cursor_end = next_boundary(text, cursor);
    if cursor < text.len() {
        spans.push(Span::styled(
            text[cursor..cursor_end].to_owned(),
            cursor_style(mode, theme),
        ));
        spans.push(Span::raw(text[cursor_end..].to_owned()));
    } else {
        spans.push(Span::styled(" ", cursor_style(mode, theme)));
    }
    Line::from(spans)
}

fn cursor_style(mode: EditMode, theme: Theme) -> Style {
    match mode {
        EditMode::Normal => theme.selected(),
        EditMode::Insert => Style::default()
            .fg(theme.selected_fg)
            .bg(theme.accent)
            .add_modifier(Modifier::BOLD),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeat_game_teaches_insert_and_completes() {
        let mut dojo = VimDojo::default();
        dojo.handle_key(KeyCode::Char('2'), KeyModifiers::NONE);
        dojo.handle_key(KeyCode::Char('i'), KeyModifiers::NONE);
        for character in REPEAT_TARGET.chars() {
            dojo.handle_key(KeyCode::Char(character), KeyModifiers::NONE);
        }
        dojo.handle_key(KeyCode::Esc, KeyModifiers::NONE);

        assert_eq!(dojo.screen, Screen::Repeat);
        assert!(dojo.repeat.is_complete());
    }

    #[test]
    fn substitute_game_applies_global_replacement() {
        let mut dojo = VimDojo::default();
        dojo.handle_key(KeyCode::Char('5'), KeyModifiers::NONE);
        dojo.handle_key(KeyCode::Char(':'), KeyModifiers::NONE);
        for character in SUBSTITUTE_COMMAND.chars() {
            dojo.handle_key(KeyCode::Char(character), KeyModifiers::NONE);
        }
        dojo.handle_key(KeyCode::Enter, KeyModifiers::NONE);

        assert!(dojo.substitute.complete);
        assert_eq!(dojo.substitute.text.matches("hate").count(), 0);
        assert_eq!(dojo.substitute.text.matches("love").count(), 2);
    }

    #[test]
    fn snake_is_a_real_long_form_level_and_advances() {
        let mut dojo = VimDojo::default();
        dojo.handle_key(KeyCode::Char('1'), KeyModifiers::NONE);

        assert_eq!(dojo.screen, Screen::Snake);
        assert_eq!(SNAKE_TARGETS, 12);
        assert!(!dojo.snake.complete);

        dojo.snake.score = SNAKE_TARGETS;
        dojo.snake.complete = true;
        dojo.tick(Duration::from_secs(1));

        assert_eq!(dojo.screen, Screen::Repeat);
    }

    #[test]
    fn text_object_and_search_levels_require_their_real_sequences() {
        let mut dojo = VimDojo::default();
        dojo.handle_key(KeyCode::Char('3'), KeyModifiers::NONE);
        for key in ['c', 'i', '"'] {
            dojo.handle_key(KeyCode::Char(key), KeyModifiers::NONE);
        }
        assert!(dojo.text_object.complete);

        dojo.tick(Duration::from_secs(1));
        assert_eq!(dojo.screen, Screen::Search);
        dojo.handle_key(KeyCode::Char('/'), KeyModifiers::NONE);
        for character in "TODO".chars() {
            dojo.handle_key(KeyCode::Char(character), KeyModifiers::NONE);
        }
        dojo.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert!(dojo.search.complete);
    }
}
