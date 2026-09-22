//! Small, opt-in Vim practice games.

use std::{cmp::Ordering, time::Duration};

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
const HUNT_WIDTH: i16 = 34;
const HUNT_HEIGHT: i16 = 10;
const HUNT_TARGETS: usize = 8;
const TEXT_OBJECT_BUFFER: &str = "const path = \"tmp/cache\";";
const SEARCH_BUFFER: [&str; 7] = [
    "  1  deploy.sh",
    "  2  notes.md",
    "  3  TODO: fix the path",
    "  4  release.txt",
    "  5  archive.md",
    "  6  TODO: ship it",
    "  7  deploy.log",
];
const SUBSTITUTE_COMMAND: &str = "%s/hate/love/g";
const WORD_HOP_TARGET: usize = 5;
const VIM_LEVEL_COUNT: usize = 6;

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
    player: Point,
    target: Point,
    score: usize,
    misses: usize,
    complete: bool,
    message: String,
}

impl Default for RepeatGame {
    fn default() -> Self {
        Self {
            player: Point { x: 3, y: 5 },
            target: Point { x: 27, y: 2 },
            score: 0,
            misses: 0,
            complete: false,
            message: "Move onto the glowing character with h j k l, then press x to delete it."
                .to_owned(),
        }
    }
}

impl RepeatGame {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn handle_key(&mut self, code: KeyCode) {
        if self.complete {
            return;
        }
        match code {
            KeyCode::Char('h') | KeyCode::Left => self.player.x = (self.player.x - 1).max(0),
            KeyCode::Char('j') | KeyCode::Down => {
                self.player.y = (self.player.y + 1).min(HUNT_HEIGHT - 1);
            }
            KeyCode::Char('k') | KeyCode::Up => self.player.y = (self.player.y - 1).max(0),
            KeyCode::Char('l') | KeyCode::Right => {
                self.player.x = (self.player.x + 1).min(HUNT_WIDTH - 1);
            }
            KeyCode::Char('x') => {
                if self.player == self.target {
                    self.score += 1;
                    if self.score >= HUNT_TARGETS {
                        self.complete = true;
                        self.message = String::from(
                            "Arena clear. x deleted every target; Vim remains delightfully terse.",
                        );
                    } else {
                        self.target = hunt_target(self.score, self.player);
                        self.message = format!(
                            "Deleted. {} targets remain; keep moving, tiny text surgeon.",
                            HUNT_TARGETS - self.score
                        );
                    }
                } else {
                    self.misses += 1;
                    self.message =
                        String::from("x deletes under the cursor. Get the glowing target first.");
                }
            }
            _ => {}
        }
    }

    fn is_complete(&self) -> bool {
        self.complete
    }
}

fn hunt_target(score: usize, player: Point) -> Point {
    let points = [
        Point { x: 27, y: 2 },
        Point { x: 8, y: 1 },
        Point { x: 21, y: 7 },
        Point { x: 31, y: 4 },
        Point { x: 13, y: 8 },
        Point { x: 3, y: 2 },
        Point { x: 25, y: 8 },
        Point { x: 16, y: 1 },
    ];
    let mut target = points[score % points.len()];
    if target == player {
        target.x = (target.x + 5).rem_euclid(HUNT_WIDTH);
    }
    target
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
    player: Point,
    target: Point,
    stage: usize,
    complete: bool,
    message: String,
}

impl Default for TextObjectGame {
    fn default() -> Self {
        Self {
            player: Point { x: 0, y: 3 },
            target: Point { x: 14, y: 3 },
            stage: 0,
            complete: false,
            message: "Navigate onto the quoted word, then use c i \" to change its inside."
                .to_owned(),
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
        match code {
            KeyCode::Char('h') | KeyCode::Left => self.player.x = (self.player.x - 1).max(0),
            KeyCode::Char('j') | KeyCode::Down => self.player.y = (self.player.y + 1).min(6),
            KeyCode::Char('k') | KeyCode::Up => self.player.y = (self.player.y - 1).max(0),
            KeyCode::Char('l') | KeyCode::Right => self.player.x = (self.player.x + 1).min(27),
            KeyCode::Char(character) if character == expected[self.stage] => {
                if self.player != self.target {
                    self.message =
                        String::from("That command needs the cursor on the quoted word.");
                    return;
                }
                self.stage += 1;
                if self.stage == expected.len() {
                    self.complete = true;
                    self.message =
                        String::from("ci\" captured the inner text. Precision editing unlocked.");
                } else {
                    self.message = format!("Target locked. Next key: {}", expected[self.stage]);
                }
            }
            KeyCode::Char(_) => {
                self.stage = 0;
                self.message = String::from("Missed. Move to the glowing word and try c i \".");
            }
            _ => {}
        }
    }
}

#[derive(Debug)]
struct SearchGame {
    query: String,
    command_mode: bool,
    match_found: bool,
    cursor: Point,
    complete: bool,
    message: String,
}

impl Default for SearchGame {
    fn default() -> Self {
        Self {
            query: String::new(),
            command_mode: false,
            match_found: false,
            cursor: Point { x: 0, y: 3 },
            complete: false,
            message: "Use /TODO to reveal the first target, then n to jump to the next one."
                .to_owned(),
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
                KeyCode::Char('h') | KeyCode::Left => self.cursor.x = (self.cursor.x - 1).max(0),
                KeyCode::Char('j') | KeyCode::Down => self.cursor.y = (self.cursor.y + 1).min(6),
                KeyCode::Char('k') | KeyCode::Up => self.cursor.y = (self.cursor.y - 1).max(0),
                KeyCode::Char('l') | KeyCode::Right => self.cursor.x = (self.cursor.x + 1).min(30),
                KeyCode::Char('/') => {
                    self.command_mode = true;
                    self.query.clear();
                    self.message = String::from("Search mode. Type TODO, then press Enter.");
                }
                KeyCode::Char('n') if self.match_found => {
                    self.complete = true;
                    self.cursor = Point { x: 5, y: 5 };
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
                    self.match_found = true;
                    self.cursor = Point { x: 5, y: 2 };
                    self.message =
                        String::from("First match found. Press n to jump to the next TODO.");
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
            "VIM DOJO  /  ESC CLOSE",
            Style::default().fg(theme.warn).add_modifier(Modifier::BOLD),
        ))];
        match self.screen {
            Screen::Menu => {
                lines.extend(menu_side_lines(theme));
            }
            Screen::Snake => lines.extend(snake_side_lines(&self.snake, theme)),
            Screen::Repeat => lines.extend(repeat_side_lines(&self.repeat, theme)),
            Screen::TextObject => lines.extend(text_object_side_lines(&self.text_object, theme)),
            Screen::Search => lines.extend(search_side_lines(&self.search, theme)),
            Screen::Substitute => lines.extend(substitute_side_lines(&self.substitute, theme)),
            Screen::WordHop => lines.extend(word_hop_side_lines(&self.word_hop, theme)),
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
        let _ = modifiers;
        if code == KeyCode::Char('r') {
            self.repeat.reset();
            return;
        }
        if code == KeyCode::Esc {
            self.screen = Screen::Menu;
            return;
        }
        self.repeat.handle_key(code);
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
                "Whack-a-mole — move and delete targets",
                theme,
            ),
            menu_item(
                self.menu_index == 2,
                "Quote rescue — navigate and use ci\"",
                theme,
            ),
            menu_item(
                self.menu_index == 3,
                "Search hunt — find TODO, then jump",
                theme,
            ),
            menu_item(
                self.menu_index == 4,
                "Word sweep — turn every hate into love",
                theme,
            ),
            menu_item(
                self.menu_index == 5,
                "Word hop — chain w b e motions",
                theme,
            ),
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

    fn repeat_lines(&self, width: u16, theme: Theme) -> Vec<Line<'static>> {
        let mut lines = game_header(
            "02",
            "WHACK-A-MOLE",
            "move the cursor, delete the glowing character",
            width,
            theme,
        );
        lines.extend([Line::from(Span::styled(
            "LIVE ARENA  /  x deletes under the cursor",
            theme.accent().add_modifier(Modifier::BOLD),
        ))]);
        for y in 0..HUNT_HEIGHT {
            lines.push(hunt_board_line(&self.repeat, y, theme));
        }
        lines.extend([
            Line::from(""),
            progress_line("DELETE", self.repeat.score, HUNT_TARGETS, theme),
            Line::from(Span::styled(
                format!("{}  /  misses {}", self.repeat.message, self.repeat.misses),
                status_style(self.repeat.is_complete(), theme),
            )),
            Line::from(""),
            key_row(&["h j k l", "move", "x", "delete", "r reset"], theme),
        ]);
        lines
    }

    fn substitute_lines(&self, width: u16, theme: Theme) -> Vec<Line<'static>> {
        let command = if self.substitute.command_mode {
            format!(":{}▌", self.substitute.command)
        } else {
            ":%s/hate/love/g".to_owned()
        };
        let mut lines = game_header(
            "05",
            "WORD SWEEP",
            "clear every hostile word with one Ex command",
            width,
            theme,
        );
        lines.extend([
            Line::from(Span::styled(
                "LIVE BUFFER  /  two targets detected",
                theme.accent().add_modifier(Modifier::BOLD),
            )),
            substitute_word_field(self.substitute.complete, theme),
            Line::from(""),
            Line::from(Span::styled(
                "COMMAND LINE",
                theme.accent().add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                command,
                cursor_style_for_command(self.substitute.command_mode, theme),
            )),
            Line::from(""),
            if self.substitute.complete {
                Line::from(Span::styled(
                    "✓ 2 MATCHES REPLACED  /  GLOBAL FLAG WORKED",
                    status_style(true, theme),
                ))
            } else {
                Line::from(Span::styled(
                    "Press : then run the command shown above.",
                    theme.dim(),
                ))
            },
            Line::from(""),
            key_row(
                &[": command mode", "Enter run", "Esc cancel", "r reset"],
                theme,
            ),
        ]);
        if self.substitute.complete {
            lines.insert(
                7,
                replacement_line(
                    "The old script says love; the new script says love.",
                    "love",
                    theme.success,
                ),
            );
        }
        lines
    }

    fn text_object_lines(&self, width: u16, theme: Theme) -> Vec<Line<'static>> {
        let mut lines = game_header(
            "03",
            "QUOTE RESCUE",
            "navigate to the quoted word, then use a real text object",
            width,
            theme,
        );
        lines.push(Line::from(Span::styled(
            "LIVE BUFFER  /  ◆ cursor   ✦ target",
            theme.accent().add_modifier(Modifier::BOLD),
        )));
        for y in 0..7 {
            lines.push(text_object_board_line(&self.text_object, y, theme));
        }
        lines.extend([
            Line::from(""),
            sequence_row(&["c", "i", "\""], self.text_object.stage, theme),
            progress_line("STEP", self.text_object.stage, 3, theme),
            Line::from(Span::styled(
                text_object_status(self.text_object.stage, self.text_object.complete),
                status_style(self.text_object.complete, theme),
            )),
            Line::from(""),
            key_row(&["c", "i", "\"", "r reset", "Esc menu"], theme),
        ]);
        lines
    }

    fn search_lines(&self, width: u16, theme: Theme) -> Vec<Line<'static>> {
        let query = if self.search.command_mode {
            format!("/{}▌", self.search.query)
        } else if self.search.complete {
            "/TODO".to_owned()
        } else {
            "/▌".to_owned()
        };
        let mut lines = game_header(
            "04",
            "SEARCH HUNT",
            "scan the buffer, then jump between live targets",
            width,
            theme,
        );
        lines.push(Line::from(Span::styled(
            "LIVE BUFFER  /  move with h j k l   /TODO then n",
            theme.accent().add_modifier(Modifier::BOLD),
        )));
        for y in 0..7 {
            lines.push(search_board_line(&self.search, y, theme));
        }
        lines.extend([
            Line::from(""),
            Line::from(Span::styled(
                "SEARCH REGISTER",
                theme.muted().add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                query,
                cursor_style_for_command(self.search.command_mode, theme),
            )),
            Line::from(Span::styled(
                search_status(
                    self.search.command_mode,
                    self.search.match_found,
                    self.search.complete,
                ),
                status_style(self.search.complete, theme),
            )),
            Line::from(""),
            key_row(&["h j k l", "move", "/TODO", "n next", "r reset"], theme),
        ]);
        lines
    }

    fn word_hop_lines(&self, width: u16, theme: Theme) -> Vec<Line<'static>> {
        let mut words = Vec::new();
        for (index, word) in self.word_hop.words.iter().enumerate() {
            let style = match index.cmp(&self.word_hop.cursor) {
                Ordering::Equal => theme.selected().add_modifier(Modifier::BOLD),
                Ordering::Less => Style::default().fg(theme.success),
                Ordering::Greater => theme.dim(),
            };
            words.push(Span::styled(format!(" {word} "), style));
            if index + 1 < self.word_hop.words.len() {
                words.push(Span::styled("  →  ", theme.muted()));
            }
        }
        let mut lines = game_header(
            "06",
            "WORD HOP",
            "move through a command buffer",
            width,
            theme,
        );
        lines.extend([
            Line::from(Span::styled(
                "COMMAND BUFFER",
                theme.accent().add_modifier(Modifier::BOLD),
            )),
            Line::from(words),
            Line::from(""),
            progress_line("JUMP", self.word_hop.cursor, WORD_HOP_TARGET, theme),
            Line::from(Span::styled(
                word_hop_status(self.word_hop.cursor, self.word_hop.used_word_motion),
                status_style(self.word_hop.is_complete(), theme),
            )),
            Line::from(""),
            key_row(&["w next", "b back", "e end", "r reset"], theme),
        ]);
        lines
    }
}

fn text_object_board_line(game: &TextObjectGame, y: i16, theme: Theme) -> Line<'static> {
    let mut spans = vec![Span::styled("  ", theme.muted())];
    let characters = TEXT_OBJECT_BUFFER.chars().collect::<Vec<_>>();
    for x in 0..28 {
        let point = Point { x, y };
        let character = if y == 3 {
            characters
                .get(usize::try_from(x).unwrap_or(usize::MAX))
                .copied()
        } else {
            None
        };
        let (symbol, style) = if game.player == point {
            (
                character.map_or("◆".to_owned(), |value| value.to_string()),
                theme.selected().add_modifier(Modifier::BOLD),
            )
        } else if game.target == point && !game.complete {
            (
                character.map_or("✦".to_owned(), |value| value.to_string()),
                theme.selected().fg(theme.warn).add_modifier(Modifier::BOLD),
            )
        } else {
            (
                character.map_or(" ".to_owned(), |value| value.to_string()),
                Style::default().fg(theme.fg),
            )
        };
        spans.push(Span::styled(symbol, style));
    }
    Line::from(spans)
}

fn search_board_line(game: &SearchGame, y: i16, theme: Theme) -> Line<'static> {
    let mut spans = vec![Span::styled("  ", theme.muted())];
    let text = SEARCH_BUFFER
        .get(usize::try_from(y).unwrap_or(usize::MAX))
        .copied()
        .unwrap_or_default();
    let characters = text.chars().collect::<Vec<_>>();
    let first_target = Point { x: 5, y: 2 };
    let second_target = Point { x: 5, y: 5 };
    for x in 0..32 {
        let point = Point { x, y };
        let character = characters
            .get(usize::try_from(x).unwrap_or(usize::MAX))
            .copied()
            .unwrap_or(' ');
        let (symbol, style) = if game.cursor == point {
            (
                character.to_string(),
                theme.selected().add_modifier(Modifier::BOLD),
            )
        } else if point == second_target && game.complete {
            (
                character.to_string(),
                Style::default()
                    .fg(theme.success)
                    .add_modifier(Modifier::BOLD),
            )
        } else if point == first_target && game.match_found {
            (
                character.to_string(),
                Style::default().fg(theme.warn).add_modifier(Modifier::BOLD),
            )
        } else {
            (character.to_string(), Style::default().fg(theme.fg))
        };
        spans.push(Span::styled(symbol, style));
    }
    Line::from(spans)
}

fn menu_side_lines(theme: Theme) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled("6 LEVELS  /  ONE KEY AT A TIME", theme.dim())),
        Line::from(""),
        key_row(&["1 snake", "2 repeat", "3 objects"], theme),
        key_row(&["4 search", "5 substitute", "6 words"], theme),
        Line::from(""),
        Line::from(Span::styled("↑↓ choose   Enter play", theme.muted())),
    ]
}

fn snake_side_lines(game: &SnakeGame, theme: Theme) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            "01 / 06  SNAKE",
            theme.accent().add_modifier(Modifier::BOLD),
        )),
        progress_line("EAT", game.score, SNAKE_TARGETS, theme),
        Line::from(Span::styled("h j k l  move", theme.fg)),
        Line::from(Span::styled("r reset   Esc menu", theme.muted())),
        Line::from(""),
        Line::from(Span::styled(
            if game.complete {
                "✓ clear"
            } else if game.game_over {
                "wall hit"
            } else {
                "live"
            },
            status_style(game.complete, theme),
        )),
    ]
}

fn repeat_side_lines(game: &RepeatGame, theme: Theme) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            "02 / 06  WHACK-A-MOLE",
            theme.accent().add_modifier(Modifier::BOLD),
        )),
        progress_line("DELETE", game.score, HUNT_TARGETS, theme),
        Line::from(Span::styled("h j k l  move", theme.fg)),
        Line::from(Span::styled("x  delete target", theme.dim())),
        Line::from(Span::styled("r reset   Esc menu", theme.muted())),
    ]
}

fn hunt_board_line(game: &RepeatGame, y: i16, theme: Theme) -> Line<'static> {
    let mut spans = vec![Span::styled("  ", theme.muted())];
    for x in 0..HUNT_WIDTH {
        let point = Point { x, y };
        let (symbol, style) = if game.player == point {
            ("◆", theme.selected().add_modifier(Modifier::BOLD))
        } else if game.target == point {
            (
                "✦",
                theme.selected().add_modifier(Modifier::BOLD).fg(theme.warn),
            )
        } else {
            ("·", Style::default().fg(theme.overlay))
        };
        spans.push(Span::styled(symbol, style));
    }
    Line::from(spans)
}

fn text_object_side_lines(game: &TextObjectGame, theme: Theme) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            "03 / 06  OBJECTS",
            theme.accent().add_modifier(Modifier::BOLD),
        )),
        sequence_row(&["c", "i", "\""], game.stage, theme),
        progress_line("STEP", game.stage, 3, theme),
        Line::from(Span::styled("h j k l  reach the quote", theme.fg)),
        Line::from(Span::styled("r reset   Esc menu", theme.muted())),
    ]
}

fn search_side_lines(game: &SearchGame, theme: Theme) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            "04 / 06  SEARCH",
            theme.accent().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            if game.command_mode {
                "/ typing"
            } else if game.complete {
                "✓ found"
            } else if game.match_found {
                "✓ first found"
            } else {
                "/ ready"
            },
            status_style(game.complete, theme),
        )),
        Line::from(Span::styled("h j k l  /TODO   n", theme.fg)),
        Line::from(Span::styled("r reset   Esc menu", theme.muted())),
    ]
}

fn substitute_side_lines(game: &SubstituteGame, theme: Theme) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            "05 / 06  EX",
            theme.accent().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled("hate  →  love", theme.warn)),
        Line::from(Span::styled(
            if game.complete {
                "✓ 2 replacements"
            } else {
                "global / g required"
            },
            status_style(game.complete, theme),
        )),
        Line::from(Span::styled(": command   Enter run", theme.fg)),
        Line::from(Span::styled("r reset   Esc menu", theme.muted())),
    ]
}

fn substitute_word_field(complete: bool, theme: Theme) -> Line<'static> {
    let (word, style) = if complete {
        (
            " LOVE ",
            Style::default()
                .fg(theme.selected_fg)
                .bg(theme.success)
                .bold(),
        )
    } else {
        (
            " HATE ",
            theme
                .selected()
                .fg(theme.error)
                .add_modifier(Modifier::BOLD),
        )
    };
    Line::from(vec![
        Span::styled("      ", theme.muted()),
        Span::styled(word, style),
        Span::styled(
            "    ✦    ",
            Style::default().fg(theme.warn).add_modifier(Modifier::BOLD),
        ),
        Span::styled(word, style),
        Span::styled("      targets", theme.muted()),
    ])
}

fn word_hop_side_lines(game: &WordHopGame, theme: Theme) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            "06 / 06  WORD HOP",
            theme.accent().add_modifier(Modifier::BOLD),
        )),
        progress_line("JUMP", game.cursor, WORD_HOP_TARGET, theme),
        Line::from(Span::styled("w next   b back", theme.fg)),
        Line::from(Span::styled("e end    r reset", theme.dim())),
        Line::from(Span::styled("Esc menu", theme.muted())),
    ]
}

fn game_header(
    level: &str,
    title: &str,
    subtitle: &str,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            format!("VIM DOJO  /  {level} OF {VIM_LEVEL_COUNT}  /  {title}"),
            theme.accent().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "─".repeat(usize::from(width.saturating_sub(1))),
            theme.muted(),
        )),
        Line::from(Span::styled(subtitle.to_owned(), theme.fg)),
        Line::from(""),
    ]
}

fn progress_line(label: &str, current: usize, total: usize, theme: Theme) -> Line<'static> {
    let total = total.max(1);
    let current = current.min(total);
    let bar_width = 18;
    let filled = bar_width * current / total;
    Line::from(vec![
        Span::styled(format!("{label:<6}"), theme.muted()),
        Span::styled("█".repeat(filled), theme.success),
        Span::styled("░".repeat(bar_width - filled), theme.overlay),
        Span::styled(format!(" {:>3}%", current * 100 / total), theme.accent()),
    ])
}

fn key_row(labels: &[&str], theme: Theme) -> Line<'static> {
    let mut spans = Vec::with_capacity(labels.len().saturating_mul(2));
    for (index, label) in labels.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(
            format!(" {label} "),
            Style::default()
                .fg(theme.accent)
                .bg(theme.overlay)
                .add_modifier(Modifier::BOLD),
        ));
    }
    Line::from(spans)
}

fn sequence_row(keys: &[&str], stage: usize, theme: Theme) -> Line<'static> {
    let mut spans = Vec::with_capacity(keys.len().saturating_mul(2));
    for (index, key) in keys.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled("  →  ", theme.muted()));
        }
        let style = match index.cmp(&stage) {
            Ordering::Less => Style::default()
                .fg(theme.selected_fg)
                .bg(theme.success)
                .add_modifier(Modifier::BOLD),
            Ordering::Equal => theme.selected(),
            Ordering::Greater => Style::default().fg(theme.muted).bg(theme.overlay),
        };
        spans.push(Span::styled(format!(" {key} "), style));
    }
    Line::from(spans)
}

fn status_style(complete: bool, theme: Theme) -> Style {
    if complete {
        theme.accent().add_modifier(Modifier::BOLD)
    } else {
        theme.dim()
    }
}

fn cursor_style_for_command(active: bool, theme: Theme) -> Style {
    if active {
        theme.selected()
    } else {
        Style::default()
            .fg(theme.fg)
            .bg(theme.overlay)
            .add_modifier(Modifier::BOLD)
    }
}

fn replacement_line(text: &str, needle: &str, color: ratatui::style::Color) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, part) in text.split(needle).enumerate() {
        if index > 0 {
            spans.push(Span::styled(
                needle.to_owned(),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ));
        }
        spans.push(Span::raw(part.to_owned()));
    }
    Line::from(spans)
}

fn text_object_status(stage: usize, complete: bool) -> &'static str {
    if complete {
        "✓ INNER TEXT SELECTED  /  clean motion"
    } else {
        match stage {
            0 => "WAITING  /  press c",
            1 => "OPERATOR SET  /  press i",
            _ => "TEXT OBJECT READY  /  press \"",
        }
    }
}

fn search_status(command_mode: bool, match_found: bool, complete: bool) -> &'static str {
    if complete {
        "✓ MATCH FOUND  /  press n to repeat"
    } else if command_mode {
        "SEARCHING  /  type TODO, then Enter"
    } else if match_found {
        "FIRST MATCH  /  press n for the next target"
    } else {
        "READY  /  press /"
    }
}

fn word_hop_status(cursor: usize, used_motion: bool) -> &'static str {
    if cursor >= WORD_HOP_TARGET {
        "✓ DESTINATION REACHED  /  chmod"
    } else if used_motion {
        "MOTION REGISTERED  /  keep hopping"
    } else {
        "READY  /  press w, b, or e"
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeat_game_teaches_insert_and_completes() {
        let mut dojo = VimDojo::default();
        dojo.handle_key(KeyCode::Char('2'), KeyModifiers::NONE);
        dojo.repeat.player = dojo.repeat.target;
        dojo.handle_key(KeyCode::Char('x'), KeyModifiers::NONE);

        assert_eq!(dojo.screen, Screen::Repeat);
        assert_eq!(dojo.repeat.score, 1);
        assert!(!dojo.repeat.is_complete());
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
        dojo.text_object.player = dojo.text_object.target;
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
        assert!(dojo.search.match_found);
        dojo.handle_key(KeyCode::Char('n'), KeyModifiers::NONE);
        assert!(dojo.search.complete);
    }
}
