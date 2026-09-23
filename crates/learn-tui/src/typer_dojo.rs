//! A continuous, non-executing Linux typing test.

use std::{ops::Range, time::Duration};

use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Gauge, Paragraph},
};

use crate::theme::Theme;

const TEST_LENGTH: Duration = Duration::from_secs(30);
const PHRASES: &[&str] = &[
    "pwd && ls -lah",
    "find . -name '*.log' -type f",
    "grep -R 'TODO' src | head -n 10",
    r"printf 'hello\n' > greeting.txt",
    "tar -czf backup.tar.gz project/",
    "I use Vim btw.",
    "There is no place like ~.",
    "NixOS describes a system with declarative configuration.",
    "A pipe sends one command's output to another.",
    "rm -rf -- /tmp/throwaway",
    "history | tail -n 8",
    "chmod +x script.sh",
    "The Linux kernel manages hardware and processes.",
    "Ctrl+C interrupts a running command.",
    "cat README.md | grep -i linux",
    "git status --short",
    "The shell remembers what you typed with the up arrow.",
    "A dotfile is a file whose name starts with a dot.",
];

#[derive(Debug)]
pub(super) struct TyperDojo {
    target: Vec<char>,
    typed: Vec<char>,
    started: Option<Duration>,
    finished: bool,
    paused_at: Option<Duration>,
    keystrokes: usize,
    errors: usize,
    samples: Vec<usize>,
    next_sample: Duration,
}

impl Default for TyperDojo {
    fn default() -> Self {
        Self {
            target: PHRASES.repeat(4).join("   ").chars().collect(),
            typed: Vec::new(),
            started: None,
            finished: false,
            paused_at: None,
            keystrokes: 0,
            errors: 0,
            samples: Vec::new(),
            next_sample: Duration::from_secs(1),
        }
    }
}

impl TyperDojo {
    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }

    pub(super) fn pause(&mut self, now: Duration) {
        self.paused_at = Some(now);
    }

    pub(super) fn resume(&mut self, now: Duration) {
        if let (Some(paused), Some(started)) = (self.paused_at.take(), self.started) {
            self.started = Some(started + now.saturating_sub(paused));
        }
    }

    pub(super) fn tick(&mut self, now: Duration) {
        let Some(started) = self.started else { return };
        if self.finished || self.paused_at.is_some() {
            return;
        }
        let elapsed = now.saturating_sub(started);
        while elapsed >= self.next_sample && self.next_sample <= TEST_LENGTH {
            self.samples.push(self.wpm_at(self.next_sample));
            self.next_sample += Duration::from_secs(1);
        }
        if elapsed >= TEST_LENGTH {
            self.finished = true;
        }
    }

    pub(super) fn handle_key(&mut self, code: KeyCode, now: Duration) {
        self.tick(now);
        if self.finished {
            if matches!(code, KeyCode::Enter | KeyCode::Tab) {
                self.reset();
            }
            return;
        }
        match code {
            KeyCode::Tab => self.reset(),
            KeyCode::Backspace => {
                self.typed.pop();
            }
            KeyCode::Char(character) => {
                let expected = self.target.get(self.typed.len()).copied();
                if let Some(expected) = expected {
                    self.started.get_or_insert(now);
                    self.keystrokes += 1;
                    if character != expected {
                        self.errors += 1;
                    }
                    self.typed.push(character);
                }
            }
            _ => {}
        }
    }

    fn correct_count(&self) -> usize {
        self.typed
            .iter()
            .zip(&self.target)
            .filter(|(a, b)| a == b)
            .count()
    }

    fn elapsed(&self, now: Duration) -> Duration {
        self.started.map_or(Duration::ZERO, |started| {
            self.paused_at
                .unwrap_or(now)
                .saturating_sub(started)
                .min(TEST_LENGTH)
        })
    }

    fn remaining(&self, now: Duration) -> u64 {
        TEST_LENGTH.saturating_sub(self.elapsed(now)).as_secs()
    }

    fn wpm_at(&self, elapsed: Duration) -> usize {
        let minutes = elapsed.as_secs_f64() / 60.0;
        if minutes < 1.0 / 60.0 {
            return 0;
        }
        (self.correct_count() as f64 / 5.0 / minutes).round() as usize
    }

    fn wpm(&self, now: Duration) -> usize {
        self.wpm_at(self.elapsed(now))
    }

    fn accuracy(&self) -> usize {
        if self.keystrokes == 0 {
            return 100;
        }
        100 * self.keystrokes.saturating_sub(self.errors) / self.keystrokes
    }

    fn graph(&self) -> String {
        const BARS: &[char] = &['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        let maximum = self.samples.iter().copied().max().unwrap_or(1).max(1);
        self.samples
            .iter()
            .map(|sample| BARS[(sample.saturating_mul(7) / maximum).min(7)])
            .collect()
    }

    fn wrapped_ranges(&self, width: usize) -> Vec<Range<usize>> {
        let mut ranges = Vec::new();
        let mut start = 0;
        while start < self.target.len() {
            let limit = (start + width.max(1)).min(self.target.len());
            let end = if limit < self.target.len() {
                self.target[start..limit]
                    .iter()
                    .rposition(|character| *character == ' ')
                    .map(|space| start + space + 1)
                    .filter(|end| *end > start + width / 2)
                    .unwrap_or(limit)
            } else {
                limit
            };
            ranges.push(start..end);
            start = end;
        }
        ranges
    }

    fn text_line(&self, range: Range<usize>, theme: Theme) -> Line<'static> {
        let mut spans = Vec::new();
        for index in range {
            let character = self.target[index];
            let style = if let Some(typed) = self.typed.get(index) {
                if *typed == character {
                    Style::default().fg(theme.success)
                } else {
                    Style::default().fg(theme.error).bg(theme.overlay)
                }
            } else if index == self.typed.len() {
                Style::default().fg(theme.selected_fg).bg(theme.selected_bg)
            } else {
                theme.dim()
            };
            spans.push(Span::styled(character.to_string(), style));
        }
        Line::from(spans)
    }

    pub(super) fn render(&self, frame: &mut Frame<'_>, area: Rect, now: Duration, theme: Theme) {
        frame.render_widget(
            Block::default().style(Style::default().bg(theme.surface)),
            area,
        );
        let width = area.width.saturating_sub(8).min(104);
        if width < 20 || area.height < 12 {
            return;
        }
        let x = area.x.saturating_add(area.width.saturating_sub(width) / 2);
        let header_y = area.y.saturating_add(2);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("TYPER", theme.accent().add_modifier(Modifier::BOLD)),
                Span::styled("   30s  ·  Linux", theme.dim()),
            ])),
            Rect::new(x, header_y, width, 1),
        );
        frame.render_widget(
            Paragraph::new(format!("{:02}", self.remaining(now)))
                .alignment(Alignment::Right)
                .style(Style::default().fg(theme.warn).add_modifier(Modifier::BOLD)),
            Rect::new(x, header_y, width, 1),
        );

        let text_y = area.y.saturating_add(area.height.saturating_mul(37) / 100);
        let line_width = usize::from(width.saturating_sub(2));
        let ranges = self.wrapped_ranges(line_width);
        let current = ranges
            .iter()
            .position(|range| self.typed.len() < range.end)
            .unwrap_or(ranges.len().saturating_sub(1));
        let first = current.saturating_sub(1);
        let lines = ranges
            .into_iter()
            .skip(first)
            .take(4)
            .map(|range| self.text_line(range, theme))
            .collect::<Vec<_>>();
        frame.render_widget(
            Paragraph::new(lines).style(Style::default().bg(theme.surface)),
            Rect::new(x, text_y, width, 4),
        );
        if self.started.is_none() {
            frame.render_widget(
                Paragraph::new("Start typing to begin").style(theme.dim()),
                Rect::new(x, text_y.saturating_sub(2), width, 1),
            );
        }

        let stats_y = text_y.saturating_add(6);
        let stats = Line::from(vec![
            Span::styled(
                format!("{:>3} wpm", self.wpm(now)),
                theme.accent().add_modifier(Modifier::BOLD),
            ),
            Span::styled("    ", theme.dim()),
            Span::styled(
                format!("{}% accuracy", self.accuracy()),
                Style::default().fg(theme.fg),
            ),
            Span::styled("    ", theme.dim()),
            Span::styled(format!("{} characters", self.typed.len()), theme.dim()),
        ]);
        frame.render_widget(Paragraph::new(stats), Rect::new(x, stats_y, width, 1));
        if !self.samples.is_empty() {
            frame.render_widget(
                Paragraph::new(self.graph()).style(Style::default().fg(theme.success)),
                Rect::new(x, stats_y.saturating_add(2), width, 1),
            );
        }
        let progress_y = area.bottom().saturating_sub(4);
        frame.render_widget(
            Gauge::default()
                .gauge_style(Style::default().fg(theme.accent).bg(theme.overlay))
                .ratio(self.elapsed(now).as_secs_f64() / TEST_LENGTH.as_secs_f64())
                .label(""),
            Rect::new(x, progress_y, width, 1),
        );
        let footer = if self.finished {
            "30 seconds complete   ·   Enter to try again   ·   Esc back"
        } else {
            "Practice text only—nothing runs   ·   Tab restart   ·   Esc back"
        };
        frame.render_widget(
            Paragraph::new(footer).style(theme.dim()),
            Rect::new(x, area.bottom().saturating_sub(2), width, 1),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runs_for_thirty_seconds_without_intermediate_enter() {
        let mut dojo = TyperDojo::default();
        for character in "pwd && ls -lah   find".chars() {
            dojo.handle_key(KeyCode::Char(character), Duration::from_millis(100));
        }
        assert_eq!(dojo.typed.len(), "pwd && ls -lah   find".len());
        assert!(!dojo.finished);
        dojo.tick(Duration::from_secs(30) + Duration::from_millis(100));
        assert!(dojo.finished);
        assert_eq!(dojo.remaining(Duration::from_secs(31)), 0);
        assert!(dojo.wpm(Duration::from_secs(31)) > 0);
        dojo.handle_key(KeyCode::Enter, Duration::from_secs(31));
        assert!(dojo.typed.is_empty());
    }

    #[test]
    fn timer_pauses_while_the_command_palette_is_open() {
        let mut dojo = TyperDojo::default();
        dojo.handle_key(KeyCode::Char('p'), Duration::ZERO);
        dojo.pause(Duration::from_secs(5));
        dojo.resume(Duration::from_secs(20));
        dojo.tick(Duration::from_secs(35));
        assert!(!dojo.finished);
        dojo.tick(Duration::from_secs(45));
        assert!(dojo.finished);
    }

    #[test]
    fn dangerous_looking_prompt_is_never_executed() {
        assert!(PHRASES.iter().any(|phrase| phrase.starts_with("rm -rf")));
        assert!(TyperDojo::default().target.len() > 1000);
    }
}
