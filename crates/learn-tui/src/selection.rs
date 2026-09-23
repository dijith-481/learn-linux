//! App-owned mouse selection for copyable lesson text.

use ratatui::{Frame, layout::Rect};

use crate::theme::Theme;

#[derive(Debug, Default)]
pub(super) struct TextSelection {
    area: Option<Rect>,
    anchor: Option<(u16, u16)>,
    end: Option<(u16, u16)>,
    cells: Vec<Vec<String>>,
}

impl TextSelection {
    pub(super) fn begin(&mut self, area: Rect, x: u16, y: u16) {
        self.area = Some(area);
        self.anchor = Some((x, y));
        self.end = Some((x, y));
        self.cells.clear();
    }

    pub(super) fn drag(&mut self, x: u16, y: u16) {
        let Some(area) = self.area else { return };
        self.end = Some((
            x.clamp(area.x, area.right().saturating_sub(1)),
            y.clamp(area.y, area.bottom().saturating_sub(1)),
        ));
    }

    pub(super) fn capture_and_highlight(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        theme: Theme,
    ) {
        if self.area != Some(area) {
            return;
        }
        let Some(anchor) = self.anchor else { return };
        let Some(end) = self.end else { return };
        self.cells.clear();
        let (first, last) = ordered(anchor, end);
        for y in area.y..area.bottom() {
            let mut row = Vec::new();
            for x in area.x..area.right() {
                let Some(cell) = frame.buffer_mut().cell_mut((x, y)) else {
                    continue;
                };
                row.push(cell.symbol().to_owned());
                if anchor != end && (y, x) >= (first.1, first.0) && (y, x) <= (last.1, last.0) {
                    cell.set_bg(theme.selected_bg).set_fg(theme.selected_fg);
                }
            }
            self.cells.push(row);
        }
    }

    pub(super) fn finish(&mut self, x: u16, y: u16) -> Option<String> {
        self.drag(x, y);
        let area = self.area?;
        let anchor = self.anchor?;
        let end = self.end?;
        if anchor == end || self.cells.is_empty() {
            self.clear();
            return None;
        }
        let (first, last) = ordered(anchor, end);
        let mut lines = Vec::new();
        for row_y in first.1..=last.1 {
            let row = self.cells.get(usize::from(row_y.saturating_sub(area.y)))?;
            let from = if row_y == first.1 { first.0 } else { area.x };
            let to = if row_y == last.1 {
                last.0
            } else {
                area.right().saturating_sub(1)
            };
            let mut line = String::new();
            for x in from..=to {
                if let Some(cell) = row.get(usize::from(x.saturating_sub(area.x))) {
                    line.push_str(cell);
                }
            }
            lines.push(line.trim_end().to_owned());
        }
        self.clear();
        let text = lines.join("\n");
        (!text.trim().is_empty()).then_some(text)
    }

    pub(super) fn clear(&mut self) {
        self.area = None;
        self.anchor = None;
        self.end = None;
        self.cells.clear();
    }
}

fn ordered(a: (u16, u16), b: (u16, u16)) -> ((u16, u16), (u16, u16)) {
    if (a.1, a.0) <= (b.1, b.0) {
        (a, b)
    } else {
        (b, a)
    }
}

pub(super) fn base64_encode(bytes: &[u8]) -> String {
    const ABC: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let a = chunk[0];
        let b = *chunk.get(1).unwrap_or(&0);
        let c = *chunk.get(2).unwrap_or(&0);
        encoded.push(char::from(ABC[usize::from(a >> 2)]));
        encoded.push(char::from(ABC[usize::from((a & 3) << 4 | b >> 4)]));
        encoded.push(if chunk.len() >= 2 {
            char::from(ABC[usize::from((b & 15) << 2 | c >> 6)])
        } else {
            '='
        });
        encoded.push(if chunk.len() >= 3 {
            char::from(ABC[usize::from(c & 63)])
        } else {
            '='
        });
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding_matches_osc52_payload() {
        assert_eq!(base64_encode(b"hello"), "aGVsbG8=");
    }

    #[test]
    fn selected_text_does_not_include_panel_outside_area() {
        let mut selection = TextSelection::default();
        selection.begin(Rect::new(2, 2, 4, 2), 2, 2);
        selection.cells = vec![
            vec!["a".into(), "b".into(), "c".into(), "d".into()],
            vec!["e".into(), "f".into(), "g".into(), "h".into()],
        ];
        assert_eq!(selection.finish(3, 3).as_deref(), Some("abcd\nef"));
    }
}
