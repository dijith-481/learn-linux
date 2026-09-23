//! Focused, non-executing Linux typing practice.

use std::time::Duration;

use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::theme::Theme;

#[derive(Debug, Clone, Copy)]
struct Prompt {
    text: &'static str,
    label: &'static str,
}

const PROMPTS: &[Prompt] = &[
    Prompt {
        text: "pwd && ls -lah",
        label: "know where you are",
    },
    Prompt {
        text: "find . -name '*.log' -type f",
        label: "find files by name",
    },
    Prompt {
        text: "grep -R 'TODO' src | head -n 10",
        label: "search and filter",
    },
    Prompt {
        text: "printf 'hello, shell\n' > greeting.txt",
        label: "save output",
    },
    Prompt {
        text: "tar -czf backup.tar.gz project/",
        label: "pack a directory",
    },
    Prompt {
        text: "I use Vim btw",
        label: "terminal folklore",
    },
    Prompt {
        text: "There is no place like ~",
        label: "home directory humor",
    },
    Prompt {
        text: "NixOS keeps system configuration declarative.",
        label: "Linux fact",
    },
    Prompt {
        text: "The Linux kernel is not the whole operating system.",
        label: "Linux fact",
    },
    Prompt {
        text: "A pipe sends stdout into the next command's stdin.",
        label: "Linux fact",
    },
    Prompt {
        text: "rm -rf -- /tmp/throwaway",
        label: "dangerous-looking text — never executed",
    },
    Prompt {
        text: "git status --short && git diff --stat",
        label: "inspect before you commit",
    },
];

#[derive(Debug, Default)]
pub(super) struct TyperDojo {
    round: usize,
    typed: Vec<char>,
    started: Option<Duration>,
    keystrokes: usize,
    errors: usize,
    samples: Vec<usize>,
}

impl TyperDojo {
    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }

    fn prompt(&self) -> Prompt {
        PROMPTS[self.round % PROMPTS.len()]
    }

    fn finished(&self) -> bool {
        self.typed.len() == self.prompt().text.chars().count()
    }

    pub(super) fn handle_key(&mut self, code: KeyCode, now: Duration) {
        if self.finished() {
            if code == KeyCode::Enter {
                self.round += 1;
                self.typed.clear();
                self.started = None;
                self.keystrokes = 0;
                self.errors = 0;
                self.samples.clear();
            }
            return;
        }
        match code {
            KeyCode::Backspace => {
                self.typed.pop();
            }
            KeyCode::Char(character) => {
                let expected = self.prompt().text.chars().nth(self.typed.len());
                if expected.is_none() {
                    return;
                }
                self.started.get_or_insert(now);
                self.keystrokes += 1;
                if expected != Some(character) {
                    self.errors += 1;
                }
                self.typed.push(character);
                if self.typed.len() % 5 == 0 {
                    self.samples.push(self.wpm(now));
                    if self.samples.len() > 24 {
                        self.samples.remove(0);
                    }
                }
            }
            KeyCode::Enter if self.prompt().text.chars().nth(self.typed.len()) == Some('\n') => {
                self.handle_key(KeyCode::Char('\n'), now);
            }
            _ => {}
        }
    }

    fn correct_count(&self) -> usize {
        self.typed
            .iter()
            .zip(self.prompt().text.chars())
            .filter(|(actual, expected)| **actual == *expected)
            .count()
    }

    fn wpm(&self, now: Duration) -> usize {
        let Some(start) = self.started else {
            return 0;
        };
        let minutes = now.saturating_sub(start).as_secs_f64() / 60.0;
        if minutes < 0.05 {
            return 0;
        }
        (self.correct_count() as f64 / 5.0 / minutes).round() as usize
    }

    fn accuracy(&self) -> usize {
        if self.keystrokes == 0 {
            return 100;
        }
        (100 * self.keystrokes.saturating_sub(self.errors)) / self.keystrokes
    }

    fn graph(&self) -> String {
        const BARS: &[char] = &['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        let maximum = self.samples.iter().copied().max().unwrap_or(1).max(1);
        self.samples
            .iter()
            .map(|sample| BARS[(sample.saturating_mul(7) / maximum).min(7)])
            .collect()
    }

    pub(super) fn render(&self, frame: &mut Frame<'_>, area: Rect, now: Duration, theme: Theme) {
        frame.render_widget(
            Block::default().style(Style::default().bg(theme.surface)),
            area,
        );
        let width = area.width.saturating_sub(4).min(78);
        let height = 17.min(area.height.saturating_sub(2));
        let card = Rect::new(
            area.x.saturating_add(area.width.saturating_sub(width) / 2),
            area.y
                .saturating_add(area.height.saturating_sub(height) / 2),
            width,
            height,
        );
        if card.width < 20 || card.height < 10 {
            return;
        }
        frame.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme.muted())
                .style(Style::default().bg(theme.surface)),
            card,
        );
        let inner = Rect::new(
            card.x.saturating_add(3),
            card.y.saturating_add(2),
            card.width.saturating_sub(6),
            card.height.saturating_sub(4),
        );
        let header = Line::from(vec![
            Span::styled("TYPER", theme.accent().add_modifier(Modifier::BOLD)),
            Span::styled("  /  ", theme.muted()),
            Span::styled(self.prompt().label, theme.dim()),
        ]);
        frame.render_widget(
            Paragraph::new(header),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );
        frame.render_widget(
            Paragraph::new(format!(
                "{:02} / {:02}",
                self.round % PROMPTS.len() + 1,
                PROMPTS.len()
            ))
            .alignment(Alignment::Right)
            .style(theme.dim()),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );
        let target = self.prompt().text.chars().collect::<Vec<_>>();
        let mut spans = Vec::with_capacity(target.len());
        for (index, character) in target.iter().enumerate() {
            let glyph = if *character == '\n' {
                "↵".to_owned()
            } else {
                character.to_string()
            };
            let style = if let Some(actual) = self.typed.get(index) {
                if *actual == *character {
                    Style::default().fg(theme.success)
                } else {
                    Style::default().fg(theme.warn).bg(theme.overlay)
                }
            } else if index == self.typed.len() {
                theme.selected()
            } else {
                theme.dim()
            };
            spans.push(Span::styled(glyph, style));
        }
        frame.render_widget(
            Paragraph::new(Line::from(spans)).wrap(Wrap { trim: false }),
            Rect::new(inner.x, inner.y.saturating_add(3), inner.width, 4),
        );
        let status = if self.finished() {
            "Enter  next line"
        } else {
            "Type the line above"
        };
        frame.render_widget(
            Paragraph::new(status).style(if self.finished() {
                Style::default().fg(theme.success)
            } else {
                theme.dim()
            }),
            Rect::new(inner.x, inner.y.saturating_add(8), inner.width, 1),
        );
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!("{:>3} WPM", self.wpm(now)), theme.accent()),
                Span::styled("    ", theme.muted()),
                Span::styled(format!("{}% accuracy", self.accuracy()), theme.fg),
                Span::styled("    ", theme.muted()),
                Span::styled(self.graph(), Style::default().fg(theme.success)),
            ])),
            Rect::new(inner.x, inner.y.saturating_add(10), inner.width, 1),
        );
        frame.render_widget(
            Paragraph::new("Practice only · nothing here runs     Esc  back").style(theme.dim()),
            Rect::new(inner.x, card.bottom().saturating_sub(3), inner.width, 1),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_tracks_accuracy_and_advances_only_after_completion() {
        let mut dojo = TyperDojo::default();
        dojo.handle_key(KeyCode::Enter, Duration::ZERO);
        assert_eq!(dojo.round, 0);
        for (index, character) in PROMPTS[0].text.chars().enumerate() {
            dojo.handle_key(
                KeyCode::Char(character),
                Duration::from_millis(index as u64 * 200),
            );
        }
        assert!(dojo.finished());
        assert_eq!(dojo.accuracy(), 100);
        assert!(dojo.wpm(Duration::from_secs(4)) > 0);
        dojo.handle_key(KeyCode::Enter, Duration::from_secs(4));
        assert_eq!(dojo.round, 1);
        assert!(dojo.typed.is_empty());
    }

    #[test]
    fn dangerous_looking_prompt_is_only_text() {
        assert!(
            PROMPTS
                .iter()
                .any(|prompt| prompt.text.starts_with("rm -rf"))
        );
    }
}
